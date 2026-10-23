//! Files the editor writes into a mesh project outside the viewer: a new
//! scene, and the copy of the package the language server resolves.

use std::fs;
use std::path::{Path, PathBuf};

use crate::project::{MeshProject, ProjectError};
use crate::shaders::LIBRARY_SOURCES;

const SCENE_TEMPLATE: &str = "import sdf_mesh::sdf_lib::{Surface, surface, sd_sphere};

fn sdf_scene(p: vec3<f32>, time: f32) -> Surface {
    return surface(sd_sphere(p, 1.0), 0u);
}
";

const PACKAGE_TOML: &str =
    "# Written by `sdf_mesh package`. Rerun it after changing the sdf_mesh version.
edition = \"2026_pre\"
root = \".\"
";

/// Writes a unit-sphere scene to `scene`, creating missing directories.
///
/// Fails, leaving nothing behind, when `scene` already exists or would not
/// open as a scene: no `wesl.toml` above it, or not a module of that
/// package (see [`MeshProject::open`]).
pub fn new_scene(scene: &Path) -> Result<PathBuf, ProjectError> {
    let failed =
        |e: std::io::Error| ProjectError(format!("cannot write `{}`: {e}", scene.display()));
    if scene.exists() {
        return Err(ProjectError(format!(
            "`{}` already exists",
            scene.display()
        )));
    }
    let created_dir = scene
        .ancestors()
        .skip(1)
        .take_while(|dir| !dir.as_os_str().is_empty() && !dir.exists())
        .last()
        .map(Path::to_path_buf);
    if let Some(parent) = scene.parent() {
        fs::create_dir_all(parent).map_err(failed)?;
    }
    fs::write(scene, SCENE_TEMPLATE).map_err(failed)?;
    MeshProject::open(scene)
        .map(|project| project.scene_file)
        .inspect_err(|_| match &created_dir {
            Some(dir) => drop(fs::remove_dir_all(dir)),
            None => drop(fs::remove_file(scene)),
        })
}

/// Writes the library modules this build embeds, with a `wesl.toml` making
/// them a package, into `dir`, overwriting earlier copies. A mesh project
/// names `dir` as its `sdf_mesh = { path = … }` dependency so the language
/// server resolves exactly what the editor compiles against.
pub fn write_package(dir: &Path) -> Result<(), ProjectError> {
    let failed = |e: std::io::Error| ProjectError(format!("cannot write `{}`: {e}", dir.display()));
    fs::create_dir_all(dir).map_err(failed)?;
    fs::write(dir.join("wesl.toml"), PACKAGE_TOML).map_err(failed)?;
    for (file, source) in LIBRARY_SOURCES {
        fs::write(dir.join(file), source).map_err(failed)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sdf_mesh_scaffold_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    const TOML: &str = "edition = \"2026_pre\"\nroot = \"shaders\"\n[dependencies]\nsdf_mesh = { path = \"sdf_mesh\" }\n";

    // Pins that a scaffolded scene in a new directory opens, and that the written package copy shows no drift.
    #[test]
    fn a_new_scene_opens_against_a_written_package() {
        let dir = scratch("new");
        fs::write(dir.join("wesl.toml"), TOML).unwrap();
        write_package(&dir.join("sdf_mesh")).unwrap();
        let scene = new_scene(&dir.join("shaders/rocks/boulder.wesl")).unwrap();
        let project = MeshProject::open(&scene).unwrap();
        assert_eq!(project.library_copy_drift(), None);
        fs::remove_dir_all(dir).unwrap();
    }

    // Pins that a refused scene leaves neither the file nor the directories made for it.
    #[test]
    fn a_refused_scene_leaves_nothing_behind() {
        let dir = scratch("refused");
        fs::write(dir.join("wesl.toml"), TOML).unwrap();
        fs::create_dir_all(dir.join("shaders")).unwrap();
        for scene in ["shaders/my-rock.wesl", "shaders/new_dir/my-rock.wesl"] {
            assert!(new_scene(&dir.join(scene)).is_err(), "{scene}");
        }
        assert_eq!(fs::read_dir(dir.join("shaders")).unwrap().count(), 0);
        fs::remove_dir_all(dir).unwrap();
    }

    // Pins that an existing file is never overwritten by a new scene.
    #[test]
    fn an_existing_scene_is_not_overwritten() {
        let dir = scratch("existing");
        fs::write(dir.join("wesl.toml"), TOML).unwrap();
        fs::create_dir_all(dir.join("shaders")).unwrap();
        fs::write(dir.join("shaders/kept.wesl"), "kept").unwrap();
        assert!(new_scene(&dir.join("shaders/kept.wesl")).is_err());
        assert_eq!(
            fs::read_to_string(dir.join("shaders/kept.wesl")).unwrap(),
            "kept"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
