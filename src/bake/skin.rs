//! The skin of a rigged bake in the form glTF stores it: per-vertex joint
//! attributes, inverse bind matrices, and each joint's keyframes relative to
//! its parent, built from the joint transforms the scene's `sdf_joint`
//! returned at the rig's keyframes.

use std::fmt;

use bevy::prelude::*;

use crate::rig::Rig;
use crate::sdf::{JointSample, Skin};

/// Why a rigged bake produced no skin.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinError(pub String);

impl fmt::Display for SkinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SkinError {}

/// One joint's transform relative to its parent at every keyframe.
#[derive(Debug, Clone, PartialEq)]
pub struct JointTrack {
    pub translations: Vec<Vec3>,
    pub rotations: Vec<Quat>,
}

/// A skin whose joint 0 is the root and the parent of every other joint, and
/// one looping animation of all of them. The rest pose, which the mesh is
/// baked in, is keyframe 0.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinAnimation {
    /// Per joint, the inverse of its model-space rest transform.
    pub inverse_binds: Vec<Mat4>,
    /// The time of each keyframe, from 0 to the loop length inclusive.
    pub times: Vec<f32>,
    /// Per joint, indexed like `inverse_binds`.
    pub tracks: Vec<JointTrack>,
}

impl SkinAnimation {
    /// Builds the skin from `samples`, `rig.joints` model-space transforms per
    /// keyframe, keyframe-major.
    ///
    /// Fails when there are fewer than `rig.sample_count()` samples, or a
    /// sample has a non-finite or zero-length rotation, which is also what an
    /// unsampled (cleared) buffer holds.
    pub fn from_samples(rig: &Rig, samples: &[JointSample]) -> Result<Self, SkinError> {
        let joints = rig.joints as usize;
        let keyframes = rig.keyframes() as usize;
        if samples.len() < joints * keyframes {
            return Err(SkinError(format!(
                "{} joint samples read back, {} expected",
                samples.len(),
                joints * keyframes
            )));
        }

        let mut tracks = vec![
            JointTrack {
                translations: Vec::with_capacity(keyframes),
                rotations: Vec::with_capacity(keyframes),
            };
            joints
        ];
        let mut inverse_binds = Vec::with_capacity(joints);
        for frame in 0..keyframes {
            let world: Vec<Transform> = (0..joints)
                .map(|joint| model_transform(&samples[frame * joints + joint], joint, frame))
                .collect::<Result<_, _>>()?;
            if frame == 0 {
                inverse_binds.extend(world.iter().map(|t| t.to_matrix().inverse()));
            }
            let root = world[0].to_matrix();
            for (joint, transform) in world.iter().enumerate() {
                let local = if joint == 0 {
                    *transform
                } else {
                    Transform::from_matrix(root.inverse() * transform.to_matrix())
                };
                let track = &mut tracks[joint];
                let rotation = match track.rotations.last() {
                    Some(previous) if previous.dot(local.rotation) < 0.0 => -local.rotation,
                    _ => local.rotation,
                };
                track.translations.push(local.translation);
                track.rotations.push(rotation);
            }
        }

        Ok(Self {
            inverse_binds,
            times: (0..rig.keyframes()).map(|frame| rig.time(frame)).collect(),
            tracks,
        })
    }

    pub fn joint_count(&self) -> usize {
        self.tracks.len()
    }
}

fn model_transform(
    sample: &JointSample,
    joint: usize,
    frame: usize,
) -> Result<Transform, SkinError> {
    let rotation = Quat::from_vec4(sample.rotation);
    let length = rotation.length();
    if !(length.is_finite() && length > 1e-6 && sample.translation.is_finite()) {
        return Err(SkinError(format!(
            "sdf_joint({joint}) at keyframe {frame} has no usable rotation"
        )));
    }
    Ok(Transform::from_translation(sample.translation).with_rotation(rotation / length))
}

/// The `JOINTS_0` and `WEIGHTS_0` values of a mesh's vertices.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinAttributes {
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
}

/// The skin attributes of the first `vertex_count` vertices.
///
/// A slot with zero weight is written as joint 0. Fails when a vertex follows,
/// with non-zero weight, a joint at or past `joints`, the scene's
/// `JOINT_COUNT`.
pub fn skin_attributes(
    skins: &[Skin],
    vertex_count: usize,
    joints: u32,
) -> Result<SkinAttributes, SkinError> {
    let mut indices = Vec::with_capacity(vertex_count);
    let mut weights = Vec::with_capacity(vertex_count);
    for skin in skins.iter().take(vertex_count) {
        let mut ids = [0u16; 4];
        let mut w = skin.weights.to_array().map(|w| w.max(0.0));
        for slot in 0..4 {
            if w[slot] == 0.0 {
                continue;
            }
            let joint = skin.joints[slot];
            if joint >= joints {
                return Err(SkinError(format!(
                    "a vertex follows joint {joint}, but JOINT_COUNT is {joints}"
                )));
            }
            ids[slot] = joint as u16;
        }
        let total: f32 = w.iter().sum();
        if total > 0.0 {
            w = w.map(|w| w / total);
        } else {
            w = [1.0, 0.0, 0.0, 0.0];
        }
        indices.push(ids);
        weights.push(w);
    }
    Ok(SkinAttributes {
        joints: indices,
        weights,
    })
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use super::*;

    fn sample(translation: Vec3, rotation: Quat) -> JointSample {
        JointSample {
            translation,
            rotation: Vec4::from(rotation),
        }
    }

    #[test]
    fn child_keyframes_are_relative_to_the_root_and_binds_invert_the_rest_pose() {
        let rig = Rig {
            joints: 2,
            seconds: 1.0,
            frames: 1,
        };
        let root = sample(Vec3::new(1.0, 0.0, 0.0), Quat::from_rotation_z(FRAC_PI_2));
        let child_rest = sample(Vec3::new(1.0, 2.0, 0.0), Quat::IDENTITY);
        let child_end = sample(Vec3::new(1.0, 2.0, 0.0), Quat::from_rotation_z(FRAC_PI_2));
        let samples = [root, child_rest, root, child_end];

        let skin = SkinAnimation::from_samples(&rig, &samples).unwrap();

        assert_eq!(skin.times, vec![0.0, 1.0]);
        assert!(skin.tracks[1].translations[0].abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-5));
        assert!(skin.tracks[1].rotations[1].dot(Quat::IDENTITY).abs() > 1.0 - 1e-5);
        assert!(skin.tracks[1].rotations[0].dot(skin.tracks[1].rotations[1]) >= 0.0);
        let rest_point = Vec3::new(1.0, 3.0, 0.0);
        let bound = skin.inverse_binds[1].transform_point3(rest_point);
        assert!(bound.abs_diff_eq(Vec3::new(0.0, 1.0, 0.0), 1e-5));
    }

    #[test]
    fn an_unsampled_joint_is_refused() {
        let rig = Rig {
            joints: 1,
            seconds: 1.0,
            frames: 1,
        };
        let samples = [JointSample::default(); 2];
        assert!(SkinAnimation::from_samples(&rig, &samples).is_err());
    }

    #[test]
    fn skin_attributes_renormalize_and_reject_undeclared_joints() {
        let skins = [Skin {
            joints: UVec4::new(2, 1, 9, 0),
            weights: Vec4::new(0.5, 0.25, 0.0, 0.0),
        }];
        let attributes = skin_attributes(&skins, 1, 3).unwrap();
        assert_eq!(attributes.joints, vec![[2, 1, 0, 0]]);
        assert_eq!(attributes.weights, vec![[2.0 / 3.0, 1.0 / 3.0, 0.0, 0.0]]);
        assert!(skin_attributes(&skins, 1, 2).is_err());
    }
}
