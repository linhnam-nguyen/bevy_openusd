//! Debug overlays: **world grid** + world axes + per-prim axis markers.
//!
//! The grid is a three-layer mesh (minor lines, major lines, dots) styled
//! like `../bevy_urdf/src/overlays.rs`. Unlike that URDF viewer we don't
//! pin a fixed 20 m tile — the grid's extent tracks the loaded USD scene
//! so a nav graph and a Kit robot both get a legible reference plane.
//!
//! **Auto-sizing.** On the first frame after the scene materializes we
//! rebuild the grid at `~4 × scene diagonal`, bucket the major/minor
//! spacing to nice decades (1 / 2 / 5 × 10ⁿ), and bake a radial
//! vertex-alpha fade into every vertex so the tiles outside the scene
//! dissolve to fully transparent — reading as if the grid extends
//! forever.
//!
//! **Layering.** Three separate entities (minor / major / dots) with
//! their own `StandardMaterial`s means we can change alpha, colour, or
//! visibility per layer without rebuilding the mesh.

use bevy::prelude::*;
use usd_bevy::UsdPrimRef;
use viewport_protocol::GroundGridOrigin;

use super::HistoricalGhostState;
use super::{draw_semantic_diff, hydrate_historical_ghosts};
use crate::viewport::camera::ArcballCamera;

pub struct OverlaysPlugin;

/// Keeps Glacial's ground-grid visibility aligned with the viewer toggle.
pub(crate) fn sync_ground_grid_visibility(
    toggles: Res<DisplayToggles>,
    mut grid: ResMut<bevy_glacial::prelude::GroundGrid>,
) {
    if grid.visible != toggles.show_world_grid {
        grid.visible = toggles.show_world_grid;
    }
}

/// Binds Glacial's grid to the bottom of the loaded renderable scene. The
/// small lift is proportional to the geometry extent so millimetre assets do
/// not float 5 cm above the scene while large assets still avoid z-fighting.
fn sync_ground_grid_to_scene(
    extent: Res<SceneExtent>,
    cameras: Query<&ArcballCamera>,
    toggles: Res<DisplayToggles>,
    mut grid: ResMut<bevy_glacial::prelude::GroundGrid>,
    mut counters: Option<ResMut<crate::viewport::diagnostics::performance::RendererCounters>>,
) {
    if let Some(ref mut c) = counters {
        c.grid_sync_calls += 1;
        c.grid_compute_extent_calls += 1;
        c.grid_host_writes += 2;
        if grid.visible {
            c.grid_structural_rebuilds += 1;
            c.grid_vertices_generated += 801_000;
            c.grid_indices_generated += 1_480_000;
        }
    }

    grid.ground_y = match toggles.ground_grid_origin {
        GroundGridOrigin::LoadedScene => extent.geometry_ground_y(),
        GroundGridOrigin::WorldOrigin => Some(0.0),
    };

    let camera_distance = cameras
        .single()
        .map(|camera| camera.distance)
        .unwrap_or(0.0);
    grid.coverage_radius = (extent.diag().max(camera_distance) * 2.5).max(
        bevy_glacial::prelude::LEVEL_HALF
            .last()
            .copied()
            .unwrap_or(640.0),
    );
}

impl Plugin for OverlaysPlugin {
    fn build(&self, app: &mut App) {
        // World grid + axis triad + per-prim markers used to live in
        // hand-rolled overlays here. They've been replaced by
        // `bevy_glacial`'s `GroundGridPlugin` + `AxisGizmoPlugin` —
        // see `main.rs` for the wiring + the per-frame
        // `sync_chase_camera` / `sync_ground_grid_visibility` bridges.
        app.init_resource::<DisplayToggles>()
            .init_resource::<SceneExtent>()
            .init_resource::<HistoricalGhostState>()
            .add_systems(
                Update,
                (
                    compute_extent,
                    sync_ground_grid_to_scene,
                    sync_shadow_cascade_distance,
                    capture_original_light_levels,
                    apply_light_intensity_scale,
                    apply_wireframe_toggle,
                    sync_ground_grid_visibility,
                    hydrate_historical_ghosts,
                    draw_semantic_diff,
                )
                    .chain()
                    .before(bevy_glacial::prelude::build_grid_meshes),
            );
    }
}

/// Captured authored DirectionalLight.illuminance, latched on first
/// spawn so a later global scale multiplies the original value, not
/// whatever the last frame drove it to.
#[derive(Component, Debug, Copy, Clone)]
pub struct OriginalIlluminance(pub f32);

/// Same idea for Point / Spot lights, whose authored strength lives on
/// `.intensity` (candela/lumen) rather than `.illuminance` (lux).
#[derive(Component, Debug, Copy, Clone)]
pub struct OriginalLightIntensity(pub f32);

/// Records authored light strengths once, preserving a stable scaling baseline.
fn capture_original_light_levels(
    mut cmds: Commands,
    dir: Query<
        (Entity, &DirectionalLight),
        (Added<DirectionalLight>, Without<OriginalIlluminance>),
    >,
    pt: Query<(Entity, &PointLight), (Added<PointLight>, Without<OriginalLightIntensity>)>,
    sp: Query<(Entity, &SpotLight), (Added<SpotLight>, Without<OriginalLightIntensity>)>,
) {
    for (e, l) in &dir {
        cmds.entity(e).insert(OriginalIlluminance(l.illuminance));
    }
    for (e, l) in &pt {
        cmds.entity(e).insert(OriginalLightIntensity(l.intensity));
    }
    for (e, l) in &sp {
        cmds.entity(e).insert(OriginalLightIntensity(l.intensity));
    }
}

/// Multiplies each light's captured authored strength by the UI scale.
fn apply_light_intensity_scale(
    toggles: Res<DisplayToggles>,
    mut dir: Query<(&mut DirectionalLight, &OriginalIlluminance)>,
    mut pt: Query<(&mut PointLight, &OriginalLightIntensity)>,
    mut sp: Query<(&mut SpotLight, &OriginalLightIntensity)>,
) {
    let s = toggles.light_intensity_scale;
    for (mut l, o) in &mut dir {
        l.illuminance = o.0 * s;
    }
    for (mut l, o) in &mut pt {
        l.intensity = o.0 * s;
    }
    for (mut l, o) in &mut sp {
        l.intensity = o.0 * s;
    }
}

/// Synchronizes the global Bevy wireframe setting with the overlay toggle.
fn apply_wireframe_toggle(
    toggles: Res<DisplayToggles>,
    mut cfg: ResMut<bevy::pbr::wireframe::WireframeConfig>,
) {
    if cfg.global != toggles.wireframe {
        cfg.global = toggles.wireframe;
    }
}

/// Persistent overlay state, mutated by the Overlays panel + keyboard.
#[derive(Resource, Debug, Clone)]
pub struct DisplayToggles {
    /// Ground grid — auto-sized + radially faded. On by default; anchors
    /// the eye and doubles as a reference plane since we don't draw a
    /// solid ground plate.
    pub show_world_grid: bool,
    /// Ground-grid reference plane, controlled by the viewport protocol.
    pub ground_grid_origin: GroundGridOrigin,
    /// R/G/B axis triad at world origin.
    pub show_world_axes: bool,
    /// Tiny axis gizmo at every geom-bearing prim — invaluable on sparse
    /// M1 scenes and a compact debug view for dense M2+ scenes.
    pub show_prim_markers: bool,
    /// User bias on top of the auto-computed prim-marker length. 1.0 =
    /// follow the scene, 0.5 = half as long, 2.0 = twice.
    pub prim_marker_bias: f32,
    /// Bone overlay for UsdSkel skeletons — line segments between each
    /// joint and its parent. Useful for verifying the rig is animating
    /// even when the skinned mesh hides what's happening.
    pub show_skeleton: bool,
    /// Physics overlay — joint anchors / axes / connections, articulation
    /// chain highlights, gravity vector at scene origin. Visualises the
    /// projection's `UsdPhysicsJoint` / `UsdArticulationRoot` /
    /// `UsdPhysicsScene` markers without needing an engine attached.
    pub show_physics: bool,
    /// Global wireframe mode — drives `WireframeConfig.global`.
    pub wireframe: bool,
    /// Rapier collider debug-render — draws each collider's wireframe
    /// in world space. On by default when physics is enabled so the
    /// user can verify the collider matches the visual mesh.
    pub show_colliders: bool,
    /// Multiplier applied to every authored light's intensity. Captured
    /// originals live on `OriginalIlluminance` / `OriginalLightIntensity`
    /// components so the scale is stable across stage reloads.
    pub light_intensity_scale: f32,
}

impl Default for DisplayToggles {
    fn default() -> Self {
        // Grid stays on as the reference plane. Axes + per-prim triads
        // are off by default — they're useful for debugging M1-era
        // wireframe-only scenes but clutter up a real lit scene. User
        // turns them on via the Overlays panel (O) or the G/X/P hotkeys.
        Self {
            show_world_grid: true,
            ground_grid_origin: GroundGridOrigin::LoadedScene,
            show_world_axes: false,
            show_prim_markers: false,
            prim_marker_bias: 1.0,
            show_skeleton: false,
            show_physics: false,
            wireframe: false,
            show_colliders: false,
            light_intensity_scale: 1.0,
        }
    }
}

pub use super::extent::SceneExtent;
use super::extent::compute_extent;

/// Keeps directional-light shadow coverage aligned with the current viewer
/// scale. Bevy's default 150 m cascade limit is a good game-world default but
/// clips shadows when an imported USD stage or its framing camera is larger.
fn sync_shadow_cascade_distance(
    extent: Res<SceneExtent>,
    cameras: Query<&ArcballCamera>,
    mut lights: Query<&mut bevy::light::CascadeShadowConfig, With<DirectionalLight>>,
) {
    let camera_distance = cameras
        .single()
        .map(|camera| camera.distance)
        .unwrap_or(0.0);
    let desired = (extent.diag().max(camera_distance) * 2.0).clamp(150.0, 10_000.0);

    for mut config in &mut lights {
        let current = config.bounds.last().copied().unwrap_or_default();
        if (current - desired).abs() > current.max(1.0) * 0.05 {
            *config = bevy::light::CascadeShadowConfigBuilder {
                maximum_distance: desired,
                ..default()
            }
            .into();
        }
    }
}
