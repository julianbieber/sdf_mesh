mod bake;
mod editor;
mod material;
mod sdf;

use bevy::{diagnostic::FrameTimeDiagnosticsPlugin, prelude::*};

fn main() -> AppExit {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "sdf_mesh".to_owned(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(FrameTimeDiagnosticsPlugin::default())
        .insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.08)))
        .add_plugins((
            sdf::SdfPlugin,
            material::TriplanarMaterialPlugin,
            bake::BakePlugin,
            editor::EditorPlugin,
        ))
        .run()
}
