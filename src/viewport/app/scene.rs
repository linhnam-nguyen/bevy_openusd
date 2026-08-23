use std::path::PathBuf;

use bevy::camera::{Hdr, PerspectiveProjection, Projection};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
#[cfg(feature = "solari")]
use bevy::{camera::CameraMainTextureUsages, render::render_resource::TextureUsages};
use bevy_glacial::prelude::{AxisGizmo, ChaseCamera};

use crate::viewport::camera::ArcballCamera;
use crate::viewport::ui_frost::{RIB_TREE, RIBBON_LEFT};

/// Tag on the viewer's fallback `DirectionalLight`.
#[derive(Component)]
pub(super) struct DefaultSun;

/// Opens the prim tree on startup to give the viewer an immediate focal panel.
pub(super) fn open_default_panel(mut ribbon: ResMut<bevy_frost::RibbonOpen>) {
    ribbon.toggle(RIBBON_LEFT, RIB_TREE);
}

/// Resolves the CLI stage argument into a stage path and its asset root.
///
/// - `cargo run` with no argument loads the self-contained spinner sample.
/// - `cargo run -- path/to/file.usda` roots relative USD references at the
///   file's parent directory.
pub(super) fn resolve_requested_asset(arg: Option<String>) -> (String, PathBuf) {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    match arg {
        None => (
            "animated_spinner.usda".to_string(),
            workspace_root.join("assets"),
        ),
        Some(raw) => {
            let path = PathBuf::from(&raw);
            let abs = if path.is_absolute() {
                path
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| workspace_root.clone())
                    .join(path)
            };
            let file = abs
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| raw.clone());
            let dir = abs
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| workspace_root.clone());
            (file, dir)
        }
    }
}

/// Spawns the default arcball camera, fallback light, and ground-support entities.
pub(super) fn spawn_camera_and_ground(mut commands: Commands) {
    // Arcball camera targeting origin; focus/distance get tuned once the
    // stage lands (stretch goal: fit-to-bounds).
    //
    // **HDR + ACES tone mapping + bloom** are essential for PBR
    // materials to look right. In Bevy 0.18 HDR is a `Hdr` marker
    // component (was a `hdr: bool` field on `Camera` previously), and
    // bloom lives in `bevy::post_process`. Without HDR, emissive
    // textures and metallic specular highlights clamp to LDR and look
    // chalky; ACES tone mapping (the curve usdview / Quick Look apply)
    // restores the filmic falloff. Bloom adds the soft edge around
    // light sources and bright reflections.
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            // The default 10cm near plane made close inspection feel like
            // "zoom stopped early" on small robotics/USD details. Keep it
            // tight so the arcball can dolly into millimetre-scale features.
            near: 0.0001,
            ..default()
        }),
        #[cfg(feature = "solari")]
        CameraMainTextureUsages::default().with(TextureUsages::STORAGE_BINDING),
        #[cfg(feature = "solari")]
        Msaa::Off,
        Hdr,
        // AgX is the modern filmic curve Blender / Krita default to.
        // ACES is more contrasty + clips highlights harder; with a
        // single 50k-lux sun ACES turned the teapot into a pure-white
        // blob. AgX rolls highlights gently, reproduces albedo more
        // faithfully, and tolerates wider exposure ranges before
        // clipping.
        Tonemapping::AgX,
        // Bloom is OFF by default — turn it on with `Bloom::default()`
        // once the user wants it. With strong direct lighting + HDR +
        // ACES, the default bloom radius blew the highlights out into
        // halos that overlapped the entire silhouette.
        // (Re-enable: add `Bloom::default()` here.)
        Transform::from_xyz(3.0, 2.5, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
        ArcballCamera {
            focus: Vec3::new(0.0, 0.4, 0.0),
            distance: 4.0,
            ..default()
        },
        // bevy_glacial's GroundGridPlugin queries `&ChaseCamera` to
        // pick the LOD level. Tag our arcball-camera entity so the
        // grid follows our actual viewport without us having to
        // adopt their full ChaseCameraPlugin (we keep our orbit
        // controls). `sync_chase_camera` mirrors focus/distance/yaw
        // every frame.
        ChaseCamera::default(),
    ));
    // World-origin axis triad — replaces our hand-rolled
    // `draw_axes` overlay system.
    commands.spawn((
        Name::new("WorldAxes"),
        Transform::default(),
        AxisGizmo::default(),
    ));
    // Indoor-overcast lux. With HDR + AgX a single 5k-lux sun reads
    // closer to a quick-look studio render than 50k did — an order of
    // magnitude lower because HDR + AgX preserve dynamic range above
    // 1.0 instead of clipping; we don't need to push raw lux.
    // Ambient stays modest so PBR keeps its contrast.
    commands.spawn((
        DirectionalLight {
            illuminance: 5_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 6.0, 3.0).looking_at(Vec3::ZERO, Vec3::Y),
        DefaultSun,
    ));
    commands.insert_resource(bevy::light::GlobalAmbientLight {
        brightness: 200.0,
        ..default()
    });
    // Ground plate intentionally gone — the WorldGrid overlay provides the
    // reference plane now, sized and faded to match the scene extent.
}
