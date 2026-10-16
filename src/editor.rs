//! What the user sees and touches around the mesh: the camera, lights and
//! orbit controls, and the on-screen status.

use bevy::{
    ecs::system::SystemParam,
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    pbr::ExtendedMaterial,
    prelude::*,
};

use crate::{
    bake::{BAKE_KEY, BakeState},
    material::{SdfSurfaceMaterial, TriplanarExtension},
    project::MeshProject,
    sdf::{
        SdfBuffers, SdfGrid, domain_aabb,
        pipeline::{SceneState, SceneStatus},
    },
    shaders::ShaderLibrary,
};

const ORBIT_SPEED: f32 = 0.006;
const ZOOM_SPEED: f32 = 0.35;
const MIN_DISTANCE: f32 = 1.2;
const MAX_DISTANCE: f32 = 24.0;

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (spawn_scene, spawn_hud))
            .add_systems(Update, (orbit_camera, update_hud));
    }
}

#[derive(Component)]
struct OrbitCamera {
    focus: Vec3,
    distance: f32,
    yaw: f32,
    pitch: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            focus: Vec3::ZERO,
            distance: 6.0,
            yaw: 0.7,
            pitch: 0.45,
        }
    }
}

#[derive(Component)]
struct HudText;

fn spawn_scene(
    mut commands: Commands,
    mut materials: ResMut<Assets<SdfSurfaceMaterial>>,
    grid: Res<SdfGrid>,
    buffers: Res<SdfBuffers>,
) {
    let material = materials.add(ExtendedMaterial {
        base: StandardMaterial {
            perceptual_roughness: 0.65,
            metallic: 0.05,
            ..default()
        },
        extension: TriplanarExtension::default(),
    });

    commands.spawn((
        Name::new("sdf_surface"),
        Mesh3d(buffers.mesh.clone()),
        MeshMaterial3d(material),
        domain_aabb(&grid),
    ));

    commands.spawn((
        Name::new("key_light"),
        DirectionalLight {
            illuminance: 9_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Name::new("fill_light"),
        DirectionalLight {
            illuminance: 2_500.0,
            ..default()
        },
        Transform::from_xyz(-6.0, -2.0, -4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    let orbit = OrbitCamera::default();
    commands.spawn((
        Name::new("editor_camera"),
        Camera3d::default(),
        orbit_transform(&orbit),
        orbit,
    ));
}

fn orbit_transform(orbit: &OrbitCamera) -> Transform {
    let rotation = Quat::from_euler(EulerRot::YXZ, orbit.yaw, -orbit.pitch, 0.0);
    let eye = orbit.focus + rotation * Vec3::new(0.0, 0.0, orbit.distance);
    Transform::from_translation(eye).looking_at(orbit.focus, Vec3::Y)
}

fn orbit_camera(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut camera: Single<(&mut OrbitCamera, &mut Transform)>,
) {
    let (orbit, transform) = &mut *camera;
    let mut changed = false;

    if mouse_buttons.pressed(MouseButton::Left) && motion.delta != Vec2::ZERO {
        orbit.yaw -= motion.delta.x * ORBIT_SPEED;
        orbit.pitch = (orbit.pitch - motion.delta.y * ORBIT_SPEED).clamp(-1.5, 1.5);
        changed = true;
    }

    if scroll.delta.y != 0.0 {
        orbit.distance =
            (orbit.distance - scroll.delta.y * ZOOM_SPEED).clamp(MIN_DISTANCE, MAX_DISTANCE);
        changed = true;
    }

    if changed {
        **transform = orbit_transform(orbit);
    }
}

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        HudText,
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(13.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            left: Val::Px(10.0),
            ..default()
        },
    ));
}

#[derive(SystemParam)]
struct SceneReport<'w> {
    project: Res<'w, MeshProject>,
    status: Res<'w, SceneStatus>,
    library: Option<Res<'w, ShaderLibrary>>,
    asset_server: Res<'w, AssetServer>,
}

impl SceneReport<'_> {
    fn line(&self) -> String {
        match self.status.get() {
            SceneState::WaitingOnImport if self.everything_loaded() => {
                "waiting on an import that does not resolve; the log names it".to_owned()
            }
            SceneState::WaitingOnImport => "loading".to_owned(),
            SceneState::Compiling => "compiling".to_owned(),
            SceneState::Compiled => "compiled".to_owned(),
            SceneState::Failed(error) => error
                .lines()
                .find(|line| line.trim_start().starts_with("error"))
                .or_else(|| error.lines().next())
                .unwrap_or("compile error")
                .to_owned(),
        }
    }

    fn everything_loaded(&self) -> bool {
        let scene_loaded = self
            .asset_server
            .get_handle::<Shader>(self.project.scene_asset_path())
            .is_some_and(|scene| self.asset_server.is_loaded_with_dependencies(&scene));
        scene_loaded
            && self
                .library
                .as_ref()
                .is_some_and(|library| library.is_loaded(&self.asset_server))
    }
}

fn update_hud(
    grid: Res<SdfGrid>,
    bake: Res<BakeState>,
    scene: SceneReport,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    mut hud: Single<&mut Text, With<HudText>>,
) {
    let fps = diagnostics
        .get(&bevy::diagnostic::FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or_default();

    let bake_line = bake.status.as_deref().unwrap_or("idle");

    hud.0 = format!(
        "{path}  -  edit and save to remesh\n\
         scene: {scene_line}\n\
         grid {res}^3   budget {budget} verts   iso {iso}\n\
         drag: orbit    wheel: zoom    {key:?}: bake glb\n\
         bake: {bake_line}\n\
         fps {fps:.0}",
        path = scene.project.scene_file.display(),
        scene_line = scene.line(),
        res = grid.resolution,
        budget = grid.max_vertices,
        iso = grid.iso,
        key = BAKE_KEY,
    );
}
