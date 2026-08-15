//! Standalone native live-editor diagnostic binary.
//!
//! Loads and visualizes OpenUSD scenes (.usd, .usda, .usdz) live using `usd_bevy`.
//!
//! Controls:
//! - Mouse: Left Drag to Orbit, Right/Middle Drag to Pan, Scroll to Zoom.
//! - Keyboard:
//!   - Space: Play / Pause animation
//!   - Left / Right Arrow: Step animation frame
//!   - 1: Toggle Proxy purpose
//!   - 2: Toggle Render purpose
//!   - 3: Toggle Guide purpose
//!   - R: Reload USD stage

use std::path::PathBuf;

use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::prelude::*;
use openusd::usd::Stage;
use usd_bevy::{DisplayPurposes, LiveStage, LiveStagePlugin, PrimEntities, StageTime, UsdPlugin};

#[derive(Resource)]
struct StagePath(String);

#[derive(Component)]
struct OrbitCamera {
    focus: Vec3,
    radius: f32,
    pitch: f32,
    yaw: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            focus: Vec3::new(0.0, 0.5, 0.0),
            radius: 4.0,
            pitch: 0.35,
            yaw: 0.6,
        }
    }
}

#[derive(Resource)]
struct AnimationPlayback {
    playing: bool,
    fps: f64,
}

impl Default for AnimationPlayback {
    fn default() -> Self {
        Self {
            playing: true,
            fps: 24.0,
        }
    }
}

#[derive(Resource, Default)]
struct ReloadRequested(bool);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let file_arg = args.iter().find(|a| !a.starts_with('-')).cloned();

    let target_file = file_arg.unwrap_or_else(|| "assets/animated_spinner.usda".to_string());
    let path = resolve_path(&target_file);
    let path_str = path.to_string_lossy().to_string();

    let title = format!(
        "usdview — {}",
        path.file_name().unwrap_or_default().to_string_lossy()
    );

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title,
                    resolution: (1400u32, 900u32).into(),
                    ..default()
                }),
                ..default()
            })
            .set(bevy::log::LogPlugin {
                filter: "warn,usdview=info,usd_bevy=info".into(),
                ..default()
            }),
    )
    .add_plugins(UsdPlugin)
    .add_plugins(LiveStagePlugin)
    .insert_resource(ClearColor(Color::srgb(0.06, 0.08, 0.12)))
    .insert_resource(StagePath(path_str.clone()))
    .init_resource::<AnimationPlayback>()
    .init_resource::<ReloadRequested>()
    .add_systems(Startup, (setup_scene, setup_camera, open_stage_system))
    .add_systems(
        Update,
        (
            orbit_camera_input,
            update_camera_transform,
            tick_animation,
            keyboard_controls,
            handle_reload_system,
            draw_grid,
        ),
    );

    app.run();
}

fn resolve_path(arg: &str) -> PathBuf {
    let path = PathBuf::from(arg);
    if path.is_absolute() && path.exists() {
        return path;
    }
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let from_manifest = manifest_dir.join(arg);
    if from_manifest.exists() {
        return from_manifest;
    }
    let cur_dir = std::env::current_dir().unwrap_or_else(|_| manifest_dir.clone());
    let from_cwd = cur_dir.join(arg);
    if from_cwd.exists() {
        return from_cwd;
    }
    from_manifest
}

fn open_stage_system(world: &mut World) {
    let path = world.resource::<StagePath>().0.clone();
    info!("opening USD stage: {path}");
    if let Some(mut cache) = world.get_resource_mut::<usd_bevy::route::material::UsdTextureCache>()
    {
        let p = std::path::PathBuf::from(&path);
        if !cache.archive_paths.contains(&p) {
            cache.archive_paths.push(p);
        }
    }
    match Stage::open(&path) {
        Ok(stage) => {
            world.insert_non_send(LiveStage::new(stage));
        }
        Err(e) => {
            error!("failed to open USD stage {path}: {e:#}");
        }
    }
}

fn handle_reload_system(world: &mut World) {
    let requested = world.resource::<ReloadRequested>().0;
    if !requested {
        return;
    }
    world.resource_mut::<ReloadRequested>().0 = false;
    let path = world.resource::<StagePath>().0.clone();

    // Despawn existing entities
    world.remove_non_send::<LiveStage>();
    if let Some(map) = world.get_resource::<PrimEntities>() {
        let entities: Vec<Entity> = map.iter().map(|(_, e)| e).collect();
        for entity in entities {
            world.despawn(entity);
        }
    }
    if let Some(mut map) = world.get_resource_mut::<PrimEntities>() {
        *map = PrimEntities::default();
    }

    if let Some(mut cache) = world.get_resource_mut::<usd_bevy::route::material::UsdTextureCache>()
    {
        let p = std::path::PathBuf::from(&path);
        if !cache.archive_paths.contains(&p) {
            cache.archive_paths.push(p);
        }
    }

    info!("reloading USD stage: {path}");
    match Stage::open(&path) {
        Ok(stage) => {
            world.insert_non_send(LiveStage::new(stage));
        }
        Err(e) => {
            error!("failed to reload USD stage {path}: {e:#}");
        }
    }
}

fn setup_scene(mut commands: Commands) {
    // Key light
    commands.spawn((
        DirectionalLight {
            illuminance: 8_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(6.0, 12.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // Fill light
    commands.spawn((
        DirectionalLight {
            illuminance: 3_000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(-8.0, 6.0, -4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // Ambient light
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 300.0,
        ..default()
    });
}

fn setup_camera(mut commands: Commands) {
    let orbit = OrbitCamera::default();
    let rot = Quat::from_euler(EulerRot::YXZ, orbit.yaw, -orbit.pitch, 0.0);
    let pos = orbit.focus + rot * Vec3::new(0.0, 0.0, orbit.radius);

    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(pos).looking_at(orbit.focus, Vec3::Y),
        orbit,
    ));
}

fn orbit_camera_input(
    mut query: Query<(&mut OrbitCamera, &Transform)>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut mouse_motion: MessageReader<MouseMotion>,
    mut mouse_wheel: MessageReader<MouseWheel>,
) {
    let Ok((mut orbit, transform)) = query.single_mut() else {
        return;
    };
    let mut delta = Vec2::ZERO;
    for ev in mouse_motion.read() {
        delta += ev.delta;
    }
    let mut scroll = 0.0;
    for ev in mouse_wheel.read() {
        scroll += ev.y;
    }

    if mouse_buttons.pressed(MouseButton::Left) {
        orbit.yaw -= delta.x * 0.005;
        orbit.pitch = (orbit.pitch + delta.y * 0.005).clamp(-1.5, 1.5);
    }
    if mouse_buttons.pressed(MouseButton::Right) || mouse_buttons.pressed(MouseButton::Middle) {
        let right = transform.right().as_vec3();
        let up = transform.up().as_vec3();
        let pan_speed = orbit.radius * 0.002;
        orbit.focus -= (right * delta.x - up * delta.y) * pan_speed;
    }
    if scroll.abs() > 0.001 {
        orbit.radius = (orbit.radius * (1.0 - scroll * 0.1)).max(0.01);
    }
}

fn update_camera_transform(mut query: Query<(&OrbitCamera, &mut Transform)>) {
    for (orbit, mut transform) in &mut query {
        let rot = Quat::from_euler(EulerRot::YXZ, orbit.yaw, -orbit.pitch, 0.0);
        let pos = orbit.focus + rot * Vec3::new(0.0, 0.0, orbit.radius);
        *transform = Transform::from_translation(pos).looking_at(orbit.focus, Vec3::Y);
    }
}

fn tick_animation(
    time: Res<Time>,
    playback: Res<AnimationPlayback>,
    mut stage_time: ResMut<StageTime>,
) {
    if playback.playing {
        stage_time.current += time.delta_secs_f64() * playback.fps;
    }
}

fn keyboard_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut playback: ResMut<AnimationPlayback>,
    mut stage_time: ResMut<StageTime>,
    mut purposes: ResMut<DisplayPurposes>,
    mut reload: ResMut<ReloadRequested>,
) {
    if keys.just_pressed(KeyCode::Space) {
        playback.playing = !playback.playing;
        info!(
            "Playback: {}",
            if playback.playing {
                "playing"
            } else {
                "paused"
            }
        );
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        stage_time.current += 1.0;
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        stage_time.current = (stage_time.current - 1.0).max(0.0);
    }
    if keys.just_pressed(KeyCode::Digit1) {
        purposes.proxy = !purposes.proxy;
        info!("Toggle proxy purpose: {}", purposes.proxy);
    }
    if keys.just_pressed(KeyCode::Digit2) {
        purposes.render = !purposes.render;
        info!("Toggle render purpose: {}", purposes.render);
    }
    if keys.just_pressed(KeyCode::Digit3) {
        purposes.guide = !purposes.guide;
        info!("Toggle guide purpose: {}", purposes.guide);
    }
    if keys.just_pressed(KeyCode::KeyR) {
        reload.0 = true;
    }
}

fn draw_grid(mut gizmos: Gizmos) {
    gizmos.grid(
        Isometry3d::from_translation(Vec3::ZERO),
        UVec2::splat(40),
        Vec2::splat(1.0),
        Color::srgba(0.3, 0.35, 0.4, 0.25),
    );
}
