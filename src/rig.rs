//! What a scene declares about its rig — how many joints it moves, over how
//! long a loop, sampled at how many keyframes — read from the scene's own
//! WESL source.

use std::fmt;

use wesl::syntax::{Expression, GlobalDeclaration, LiteralExpression, TranslationUnit};

/// The most joints a rig may declare.
pub const MAX_JOINTS: u32 = 64;

/// The most joint transforms one bake samples: joints times keyframes,
/// counting the keyframe that closes the loop.
pub const MAX_SAMPLES: u32 = MAX_JOINTS * 257;

/// The rig of a scene that declares `JOINT_COUNT`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rig {
    pub joints: u32,
    pub seconds: f32,
    pub frames: u32,
}

impl Rig {
    /// Keyframes sampled per joint: `frames` over the loop, plus the one at
    /// `seconds` that closes it.
    pub fn keyframes(&self) -> u32 {
        self.frames + 1
    }

    pub fn sample_count(&self) -> u32 {
        self.joints * self.keyframes()
    }

    pub fn time(&self, frame: u32) -> f32 {
        self.seconds * frame as f32 / self.frames as f32
    }
}

/// Why a scene's rig cannot be baked.
#[derive(Debug, Clone, PartialEq)]
pub struct RigError(pub String);

impl fmt::Display for RigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RigError {}

/// Whether `source` defines a module-scope `fn sdf_joint`. A source that does
/// not parse defines none.
pub fn defines_sdf_joint(source: &str) -> bool {
    source.parse::<TranslationUnit>().is_ok_and(|unit| {
        unit.global_declarations.iter().any(
            |decl| matches!(&**decl, GlobalDeclaration::Function(f) if *f.ident.name() == "sdf_joint"),
        )
    })
}

/// The rig `source` declares, or `None` for a scene without `JOINT_COUNT`,
/// which bakes static.
///
/// A scene with `JOINT_COUNT` must also declare `ANIMATION_SECONDS` and
/// `ANIMATION_FRAMES` and define `fn sdf_joint`. Each constant must be
/// initialized with a plain literal: `JOINT_COUNT` and `ANIMATION_FRAMES`
/// integers of at least one, `ANIMATION_SECONDS` a positive number. The
/// error names the constant that breaks a rule, or the limit
/// ([`MAX_JOINTS`], [`MAX_SAMPLES`]) that the rig exceeds.
pub fn read_rig(source: &str) -> Result<Option<Rig>, RigError> {
    let unit: TranslationUnit = source
        .parse()
        .map_err(|e| RigError(format!("the scene does not parse: {e}")))?;
    let Some(joints) = literal(&unit, "JOINT_COUNT") else {
        return Ok(None);
    };
    let joints = count(joints, "JOINT_COUNT")?;
    let frames = count(
        literal(&unit, "ANIMATION_FRAMES").ok_or_else(|| missing("ANIMATION_FRAMES"))?,
        "ANIMATION_FRAMES",
    )?;
    let seconds =
        match literal(&unit, "ANIMATION_SECONDS").ok_or_else(|| missing("ANIMATION_SECONDS"))? {
            LiteralExpression::AbstractFloat(v) => v as f32,
            LiteralExpression::F32(v) => v,
            LiteralExpression::AbstractInt(v) => v as f32,
            _ => return Err(RigError("ANIMATION_SECONDS is not a number".to_owned())),
        };
    if !(seconds.is_finite() && seconds > 0.0) {
        return Err(RigError("ANIMATION_SECONDS must be positive".to_owned()));
    }
    if !defines_sdf_joint(source) {
        return Err(RigError(
            "the scene declares JOINT_COUNT but defines no fn sdf_joint".to_owned(),
        ));
    }
    if joints > MAX_JOINTS {
        return Err(RigError(format!(
            "JOINT_COUNT {joints} exceeds the limit of {MAX_JOINTS}"
        )));
    }
    let rig = Rig {
        joints,
        seconds,
        frames,
    };
    if rig.sample_count() > MAX_SAMPLES {
        return Err(RigError(format!(
            "{joints} joints at {} keyframes exceed the limit of {MAX_SAMPLES} samples",
            rig.keyframes()
        )));
    }
    Ok(Some(rig))
}

fn missing(name: &str) -> RigError {
    RigError(format!(
        "the scene declares JOINT_COUNT but no literal const {name}"
    ))
}

fn literal(unit: &TranslationUnit, name: &str) -> Option<LiteralExpression> {
    unit.global_declarations
        .iter()
        .find_map(|decl| match &**decl {
            GlobalDeclaration::Declaration(d) if d.kind.is_const() && *d.ident.name() == name => {
                match d.initializer.as_deref() {
                    Some(Expression::Literal(literal)) => Some(*literal),
                    _ => None,
                }
            }
            _ => None,
        })
}

fn count(literal: LiteralExpression, name: &str) -> Result<u32, RigError> {
    let value = match literal {
        LiteralExpression::AbstractInt(v) => u32::try_from(v).ok(),
        LiteralExpression::U32(v) => Some(v),
        LiteralExpression::I32(v) => u32::try_from(v).ok(),
        _ => None,
    };
    value
        .filter(|v| *v >= 1)
        .ok_or_else(|| RigError(format!("{name} must be an integer of at least 1")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RIGGED: &str = "
        const ANIMATION_SECONDS: f32 = 2.0;
        const ANIMATION_FRAMES: u32 = 30u;
        const JOINT_COUNT: u32 = 3;
        fn sdf_joint(i: u32, time: f32) -> f32 { return time; }
    ";

    #[test]
    fn a_rigged_scene_reads_its_constants() {
        let rig = read_rig(RIGGED).unwrap().unwrap();
        assert_eq!(
            rig,
            Rig {
                joints: 3,
                seconds: 2.0,
                frames: 30
            }
        );
        assert_eq!(rig.sample_count(), 93);
        assert_eq!(rig.time(30), 2.0);
        assert!(defines_sdf_joint(RIGGED));
    }

    #[test]
    fn a_scene_without_joint_count_has_no_rig() {
        let source = "const ANIMATION_SECONDS: f32 = 2.0;\nconst ANIMATION_FRAMES: u32 = 30;\n";
        assert_eq!(read_rig(source), Ok(None));
        assert!(!defines_sdf_joint(source));
    }

    #[test]
    fn a_rig_missing_a_constant_or_sdf_joint_is_refused() {
        let no_frames = RIGGED.replace("ANIMATION_FRAMES", "FRAMES");
        assert!(
            read_rig(&no_frames)
                .unwrap_err()
                .0
                .contains("ANIMATION_FRAMES")
        );
        let no_joint_fn = RIGGED.replace("fn sdf_joint", "fn other");
        assert!(read_rig(&no_joint_fn).unwrap_err().0.contains("sdf_joint"));
        let computed = RIGGED.replace("30u", "15u * 2u");
        assert!(
            read_rig(&computed)
                .unwrap_err()
                .0
                .contains("ANIMATION_FRAMES")
        );
        let too_many = RIGGED.replace("= 3;", "= 65;");
        assert!(read_rig(&too_many).unwrap_err().0.contains("limit"));
    }
}
