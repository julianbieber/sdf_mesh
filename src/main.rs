//! Entry point: the command line, which opens the viewer on a scene or
//! scaffolds files into a mesh project.

mod bake;
mod editor;
mod material;
mod project;
mod scaffold;
mod sdf;
mod shaders;

use std::ffi::OsString;
use std::path::Path;

use bevy::{asset::AssetMetaCheck, diagnostic::FrameTimeDiagnosticsPlugin, prelude::*};

use project::{MeshProject, ProjectError};

const USAGE: &str = "usage:
  sdf_mesh <scene.wesl>        open the viewer on a scene; edits hot reload
  sdf_mesh new <scene.wesl>    write a unit-sphere scene into a mesh project
  sdf_mesh package <dir>       write this build's sdf_mesh WESL package into <dir>";

fn main() -> AppExit {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let args: Vec<&OsString> = args.iter().collect();
    let outcome = match args.as_slice() {
        [command, scene] if *command == "new" => {
            scaffold::new_scene(Path::new(scene)).map(|scene| println!("wrote {}", scene.display()))
        }
        [command, dir] if *command == "package" => scaffold::write_package(Path::new(dir))
            .map(|()| println!("wrote the sdf_mesh package to {}", Path::new(dir).display())),
        [scene] if !scene.to_string_lossy().starts_with('-') => {
            return MeshProject::open(Path::new(scene)).map_or_else(exit_with, view);
        }
        _ => {
            eprintln!("{USAGE}");
            return AppExit::from_code(2);
        }
    };
    outcome.map_or_else(exit_with, |()| AppExit::Success)
}

fn exit_with(error: ProjectError) -> AppExit {
    eprintln!("{error}");
    AppExit::from_code(1)
}

fn view(project: MeshProject) -> AppExit {
    for name in project.foreign_dependencies() {
        eprintln!(
            "warning: `{}` declares the dependency `{name}`, which the editor does not provide; \
             a scene importing it will not compile",
            project.toml_dir.join("wesl.toml").display()
        );
    }
    if let Some((copy, drifted)) = project.library_copy_drift() {
        eprintln!(
            "warning: the language server reads `{}` from `{}`, which differs from the package \
             this editor embeds ({}); run `sdf_mesh package {}` to rewrite it",
            shaders::PACKAGE,
            copy.display(),
            drifted.join(", "),
            copy.display()
        );
    }

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: project.package_root.to_string_lossy().into_owned(),
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "sdf_mesh".to_owned(),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins(FrameTimeDiagnosticsPlugin::default())
        .insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.08)))
        .insert_resource(project)
        .add_plugins((
            shaders::ShaderPackagePlugin,
            sdf::SdfPlugin,
            material::TriplanarMaterialPlugin,
            bake::BakePlugin,
            editor::EditorPlugin,
        ))
        .run()
}
