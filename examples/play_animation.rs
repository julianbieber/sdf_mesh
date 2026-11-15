//! Plays the first animation of a baked glTF the way a game would: load the
//! file's default scene, hand its animation to the `AnimationPlayer` the
//! loader spawned, loop it. Nothing here knows how the file was made.
//!
//! `cargo run --example play_animation -- <file.glb>`

use std::path::PathBuf;

use bevy::{prelude::*, world_serialization::WorldInstanceReady};

#[derive(Component)]
struct AnimationToPlay {
    graph: Handle<AnimationGraph>,
    index: AnimationNodeIndex,
}

fn main() -> AppExit {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: cargo run --example play_animation -- <file.glb>");
        return AppExit::from_code(2);
    };
    let path = match std::fs::canonicalize(&path) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("cannot open {}: {error}", path.display());
            return AppExit::from_code(1);
        }
    };
    let dir = path.parent().expect("a file has a parent").to_owned();
    let file = path
        .file_name()
        .expect("a file has a name")
        .to_string_lossy()
        .into_owned();

    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin {
            file_path: dir.to_string_lossy().into_owned(),
            ..default()
        }))
        .insert_resource(GlobalAmbientLight {
            brightness: 400.0,
            ..default()
        })
        .insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.08)))
        .add_systems(
            Startup,
            move |mut commands: Commands,
                  asset_server: Res<AssetServer>,
                  mut graphs: ResMut<Assets<AnimationGraph>>| {
                let (graph, index) = AnimationGraph::from_clip(
                    asset_server.load(GltfAssetLabel::Animation(0).from_asset(file.clone())),
                );
                commands
                    .spawn((
                        AnimationToPlay {
                            graph: graphs.add(graph),
                            index,
                        },
                        WorldAssetRoot(
                            asset_server.load(GltfAssetLabel::Scene(0).from_asset(file.clone())),
                        ),
                    ))
                    .observe(play_when_ready);
                commands.spawn((
                    Camera3d::default(),
                    Transform::from_xyz(0.4, 0.9, 4.2).looking_at(Vec3::ZERO, Vec3::Y),
                ));
                commands.spawn((
                    DirectionalLight {
                        illuminance: 9_000.0,
                        ..default()
                    },
                    Transform::from_xyz(4.0, 8.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
                ));
            },
        )
        .run()
}

fn play_when_ready(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    animations: Query<&AnimationToPlay>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let Ok(animation) = animations.get(ready.entity) else {
        return;
    };
    for child in children.iter_descendants(ready.entity) {
        if let Ok(mut player) = players.get_mut(child) {
            player.play(animation.index).repeat();
            commands
                .entity(child)
                .insert(AnimationGraphHandle(animation.graph.clone()));
        }
    }
}
