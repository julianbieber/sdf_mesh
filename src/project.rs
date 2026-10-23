//! The mesh project the editor is opened on: the scene file it was started
//! with, and the `wesl.toml` that decides what `package::` means for it, read
//! through the wesl crate's implementation of that file's rules.

use std::fmt;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use wesl::package::PackageModule;
use wesl::toml_cfg::{DependenciesConfig, DependencySpec, WeslToml, scan_from_config};

use crate::shaders::{LIBRARY_SOURCES, PACKAGE};

/// Why a scene file could not be opened.
#[derive(Debug)]
pub struct ProjectError(pub String);

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProjectError {}

/// A scene inside its mesh project.
#[derive(Resource, Debug, Clone)]
pub struct MeshProject {
    /// The directory holding `wesl.toml`, absolute.
    pub toml_dir: PathBuf,
    /// The package root, absolute; the editor's default asset root while it is open.
    pub package_root: PathBuf,
    /// The scene file, absolute.
    pub scene_file: PathBuf,
    /// The scene's module path below `package::`, one entry per segment.
    pub scene_module: Vec<String>,
    config: WeslToml,
}

impl MeshProject {
    /// Opens the scene at `scene`, in the project of the nearest `wesl.toml` in
    /// one of its ancestor directories.
    ///
    /// `scene` may be relative to the current directory. Fails, with a message
    /// naming the file, when it does not exist, is not a `.wesl` file, has no
    /// `wesl.toml` above it, that file does not parse, or the `wesl.toml` rules
    /// (`root`, `include`, `exclude`, identifier names) do not make it a module
    /// of the package.
    pub fn open(scene: &Path) -> Result<Self, ProjectError> {
        let scene_file = std::fs::canonicalize(scene)
            .map_err(|e| ProjectError(format!("cannot open scene `{}`: {e}", scene.display())))?;
        if !scene_file.is_file() || scene_file.extension().is_none_or(|ext| ext != "wesl") {
            return Err(ProjectError(format!(
                "scene `{}` is not a .wesl file",
                scene_file.display()
            )));
        }
        let toml_dir = scene_file
            .ancestors()
            .skip(1)
            .find(|dir| dir.join("wesl.toml").is_file())
            .ok_or_else(|| {
                ProjectError(format!(
                    "no wesl.toml in any directory above `{}`",
                    scene_file.display()
                ))
            })?
            .to_path_buf();
        let toml_path = toml_dir.join("wesl.toml");
        let config = WeslToml::from_file(&toml_path)
            .map_err(|e| ProjectError(format!("`{}`: {e}", toml_path.display())))?;
        let package_root = toml_dir.join(&config.package.root);
        let package_root = std::fs::canonicalize(&package_root).map_err(|e| {
            ProjectError(format!(
                "package root `{}` of `{}`: {e}",
                package_root.display(),
                toml_path.display()
            ))
        })?;

        let not_a_module = || {
            ProjectError(format!(
                "scene `{}` is not a module of the package rooted at `{}`; check root, include \
                 and exclude in `{}`, and that every path segment is a WESL identifier",
                scene_file.display(),
                package_root.display(),
                toml_path.display()
            ))
        };
        let scene_module: Vec<String> = scene_file
            .with_extension("")
            .strip_prefix(&package_root)
            .map_err(|_| not_a_module())?
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let scan = scan_from_config(PACKAGE, &toml_dir, &config)
            .map_err(|e| ProjectError(format!("cannot scan `{}`: {e}", toml_dir.display())))?;
        if !contains_module(&scan.module.submodules, &scene_module) {
            return Err(not_a_module());
        }

        Ok(Self {
            toml_dir,
            package_root,
            scene_file,
            scene_module,
            config,
        })
    }

    /// The scene's asset path, relative to the package root.
    pub fn scene_asset_path(&self) -> String {
        format!("{}.wesl", self.scene_module.join("/"))
    }

    /// Where a bake of this scene is written: `bake/<scene module path>.glb`
    /// beside the project's `wesl.toml`.
    pub fn bake_file(&self) -> PathBuf {
        self.toml_dir
            .join("bake")
            .join(format!("{}.glb", self.scene_module.join("/")))
    }

    /// The library modules that the copy of this package named by the
    /// project's `sdf_mesh = { path = … }` dependency lacks or holds a
    /// different version of, with that copy's directory. `None` when the
    /// project declares no such dependency, or the copy matches the modules
    /// the editor embeds. A copy whose `wesl.toml` is missing or unreadable
    /// counts as lacking every module.
    pub fn library_copy_drift(&self) -> Option<(PathBuf, Vec<&'static str>)> {
        let DependenciesConfig::Manual(dependencies) = &self.config.dependencies else {
            return None;
        };
        let Some(DependencySpec::Path(path)) = dependencies.get(PACKAGE) else {
            return None;
        };
        let copy = self.toml_dir.join(path);
        let root = WeslToml::from_file(copy.join("wesl.toml"))
            .ok()
            .map(|config| copy.join(config.package.root));
        let drifted: Vec<&'static str> = LIBRARY_SOURCES
            .iter()
            .filter(|(file, source)| {
                root.as_ref()
                    .and_then(|root| std::fs::read_to_string(root.join(file)).ok())
                    .is_none_or(|on_disk| on_disk != *source)
            })
            .map(|(file, _)| *file)
            .collect();
        (!drifted.is_empty()).then_some((copy, drifted))
    }

    /// The names of dependencies other than this package that the project's
    /// `wesl.toml` declares, including this package declared under another
    /// name. The editor resolves none of them, so a scene importing one never
    /// compiles.
    pub fn foreign_dependencies(&self) -> Vec<String> {
        let DependenciesConfig::Manual(dependencies) = &self.config.dependencies else {
            return Vec::new();
        };
        let mut names: Vec<String> = dependencies
            .keys()
            .filter(|name| name.as_str() != PACKAGE)
            .cloned()
            .collect();
        names.sort();
        names
    }
}

fn contains_module(modules: &[PackageModule], path: &[String]) -> bool {
    let Some((first, rest)) = path.split_first() else {
        return false;
    };
    modules
        .iter()
        .find(|module| &module.name == first)
        .is_some_and(|module| rest.is_empty() || contains_module(&module.submodules, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sdf_mesh_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, source) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, source).unwrap();
        }
        dir
    }

    const TOML: &str = "edition = \"2026_pre\"\nroot = \"shaders\"\n[dependencies]\nsdf_mesh = { path = \"..\" }\nother = { path = \"../other\" }\n";

    // Pins that a nested scene is named by its path below the nearest wesl.toml's package root.
    #[test]
    fn a_nested_scene_is_named_below_the_package_root() {
        let dir = project_dir(
            "nested",
            &[("wesl.toml", TOML), ("shaders/rocks/boulder.wesl", "")],
        );
        let project = MeshProject::open(&dir.join("shaders/rocks/boulder.wesl")).unwrap();
        assert_eq!(project.scene_module, ["rocks", "boulder"]);
        assert_eq!(project.scene_asset_path(), "rocks/boulder.wesl");
        assert_eq!(
            project.package_root,
            dir.join("shaders").canonicalize().unwrap()
        );
        assert_eq!(project.foreign_dependencies(), ["other"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    // Pins that a bake lands beside the project's wesl.toml, named after the scene module.
    #[test]
    fn a_bake_is_written_into_the_scene_project() {
        let dir = project_dir(
            "bake",
            &[("wesl.toml", TOML), ("shaders/rocks/boulder.wesl", "")],
        );
        let project = MeshProject::open(&dir.join("shaders/rocks/boulder.wesl")).unwrap();
        assert_eq!(
            project.bake_file(),
            dir.canonicalize().unwrap().join("bake/rocks/boulder.glb")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    // Pins that the example project, whose dependency is this checkout, sees no drift.
    #[test]
    fn the_example_project_library_copy_matches_the_embedded_one() {
        let project = MeshProject::open(Path::new("example/shaders/scene.wesl")).unwrap();
        assert_eq!(project.library_copy_drift(), None);
    }

    // Pins that a copy of the library is compared module by module against the embedded one.
    #[test]
    fn an_edited_library_copy_is_reported_by_module() {
        let (types, types_source) = LIBRARY_SOURCES[0];
        let (lib, _) = LIBRARY_SOURCES[1];
        let dir = project_dir(
            "drift",
            &[
                (
                    "project/wesl.toml",
                    TOML.replace("\"..\"", "\"../copy\"").as_str(),
                ),
                ("project/shaders/scene.wesl", ""),
                ("copy/wesl.toml", "edition = \"2026_pre\"\nroot = \".\"\n"),
                (&format!("copy/{types}"), types_source),
                (&format!("copy/{lib}"), "// edited\n"),
            ],
        );
        let project = MeshProject::open(&dir.join("project/shaders/scene.wesl")).unwrap();
        let (_, drifted) = project.library_copy_drift().unwrap();
        assert_eq!(drifted, [lib]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    // Pins that a file the wesl.toml rules do not make a module is refused at startup.
    #[test]
    fn a_scene_outside_the_package_or_not_an_identifier_is_refused() {
        let dir = project_dir(
            "outside",
            &[
                ("wesl.toml", TOML),
                ("loose.wesl", ""),
                ("shaders/my-rock.wesl", ""),
            ],
        );
        for scene in ["loose.wesl", "shaders/my-rock.wesl"] {
            let error = MeshProject::open(&dir.join(scene)).unwrap_err();
            assert!(error.0.contains("not a module"), "{scene}: {error}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    // Pins that a scene with no wesl.toml above it is refused with an error naming it.
    #[test]
    fn a_scene_without_wesl_toml_is_refused() {
        let dir = project_dir("no_toml", &[("scene.wesl", "")]);
        let error = MeshProject::open(&dir.join("scene.wesl")).unwrap_err();
        assert!(error.0.contains("no wesl.toml"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
