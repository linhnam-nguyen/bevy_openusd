//! `bevy_glacial` gizmo demo — a single accent-coloured cube on a
//! flat ground, manipulated with the upstream `transform-gizmo-bevy`
//! crate (translate + rotate + scale handles, full picking / drag /
//! draw built in).
//!
//! Mouse:
//!   * MMB-drag             pan the camera focus (XZ plane)
//!   * MMB+LMB / MMB+RMB    lift the focus (vertical Y, drag up/down)
//!   * LMB+RMB-drag         orbit yaw + pitch
//!   * Scroll               zoom (log-smoothed)
//!   * MMB × 2              snap focus to cursor's ground point
//!   * LMB-drag a handle    translate / rotate / scale the cube
//!
//! Keyboard (forwarded by the gizmo crate's hotkey resource):
//!   * G                translate-only
//!   * R                rotate-only
//!   * S                scale-only
//!   * X / Y / Z        constrain to that axis
//!   * Esc              clear mode override
//!
//! This example uses the `GlacialPlugins` bundle for brevity. To pick
//! individual pieces, add them directly — e.g. `.add_plugins(
//! TransformGizmoPlugin)` for gizmo-only, or `GlacialPlugins.build()
//! .disable::<GroundGridPlugin>()` to drop the grid.

use bevy::light::CascadeShadowConfigBuilder;
use bevy::prelude::*;
use bevy_glacial::prelude::*;

fn main() {
    // Restore the saved window geometry from the last run, falling
    // back to a 1280×800 default if there's nothing on disk yet.
    let geometry = WindowGeometry::load("bevy_glacial");

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(geometry.to_window("bevy_glacial gizmo demo")),
            ..default()
        }))
        .add_plugins(GlacialPlugins)
        // Persists window size + position back to disk on every
        // resize / move. Not part of GlacialPlugins because the
        // app name is host-specific.
        .add_plugins(WindowSettingsPlugin::new("bevy_glacial"))
        // Dark navy clear colour — keeps the scene contrasty for
        // the cube + grid while leaning slightly blue so the grid's
        // cool-white lines feel cohesive against the sky.
        .insert_resource(ClearColor(Color::srgb(0.05, 0.08, 0.14)))
        .insert_resource(GizmoOptions {
            hotkeys: Some(GizmoHotkeys::default()),
            ..default()
        })
        .init_resource::<GizmoAutoScale>()
        .add_systems(Startup, setup_scene)
        .add_systems(Update, auto_scale_gizmo_to_target)
        .run();
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // No ground plane — the LOD grid is fully self-contained and
    // sits on the y = 0 plane on its own. This example demonstrates
    // that `bevy_glacial` does not force a world on the host app.

    // The subject of the gizmo: an accent-tinted 2 m cube.
    commands.spawn((
        Name::new("GizmoSubject"),
        Mesh3d(meshes.add(Cuboid::new(2.0, 2.0, 2.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.65, 0.54, 0.98),
            perceptual_roughness: 0.55,
            ..default()
        })),
        Transform::from_xyz(0.0, 1.0, 0.0),
        GizmoTarget::default(),
    ));

    // Sun + ambient.
    let sun_shadow = CascadeShadowConfigBuilder {
        num_cascades: 1,
        minimum_distance: 0.1,
        maximum_distance: 100.0,
        first_cascade_far_bound: 100.0,
        overlap_proportion: 0.0,
    }
    .build();
    commands.spawn((
        Name::new("Sun"),
        Transform::from_xyz(5.0, 50.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        DirectionalLight {
            illuminance: 10_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        sun_shadow,
    ));

    // Camera with the chase rig + the GizmoCamera marker so the
    // transform-gizmo plugin knows which view to draw against.
    //
    // To make the camera follow another entity, attach a
    // `FollowTarget { target, offset, lerp_speed }` on this entity.
    let chase = ChaseCamera {
        // No ground in this demo — let the camera orbit below the
        // horizon to look at the cube from underneath.
        min_elevation: -89f32.to_radians(),
        ..Default::default()
    };
    let mut tr = Transform::default();
    apply_rig(&chase, &mut tr);
    commands.spawn((
        Name::new("Camera"),
        Camera3d::default(),
        tr,
        AmbientLight {
            color: Color::WHITE,
            brightness: 120.0,
            ..default()
        },
        chase,
        GizmoCamera,
    ));
}
