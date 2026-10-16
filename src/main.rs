//! Entry point: opens the viewer on the scene file named on the command line,
//! or on the example project's scene when none is.

mod bake;
mod editor;
mod material;
mod project;
mod sdf;
mod shaders;

use std::path::{Path, PathBuf};

use bevy::{
    asset::{AssetMetaCheck, io::AssetSourceBuilder},
    diagnostic::FrameTimeDiagnosticsPlugin,
    prelude::*,
};

use project::MeshProject;

fn main() -> AppExit {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scene = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("example/shaders/scene.wesl"));
    let project = match MeshProject::open(&scene) {
        Ok(project) => project,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    for name in project.foreign_dependencies() {
        eprintln!(
            "warning: `{}` declares the dependency `{name}`, which the editor does not provide; \
             a scene importing it will not compile",
            project.toml_dir.join("wesl.toml").display()
        );
    }

    App::new()
        .register_asset_source(
            material::EDITOR_ASSETS,
            AssetSourceBuilder::platform_default(
                &manifest_dir.join("assets").to_string_lossy(),
                None,
            ),
        )
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
