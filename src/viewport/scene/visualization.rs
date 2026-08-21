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
use viewport_protocol::{GroundGridOrigin, RenderMode, RendererConfiguration};

use super::HistoricalGhostState;
use super::{draw_semantic_diff, hydrate_historical_ghosts};
use crate::viewport::camera::ArcballCamera;
use crate::viewport::diagnostics::performance::GroundGridDecisionHelper;

pub struct OverlaysPlugin;

/// Keeps Glacial's ground-grid visibility aligned with the viewer toggle.
pub(crate) fn sync_ground_grid_visibility(
    toggles: Res<DisplayToggles>,
    mut grid: ResMut<bevy_glacial::prelude::GroundGrid>,
) {
    if grid.visible != toggles.renderer.grid {
        grid.visible = toggles.renderer.grid;
    }
}

fn sync_ground_grid_to_scene(
    extent: Res<SceneExtent>,
    cameras: Query<&ArcballCamera>,
    toggles: Res<DisplayToggles>,
    mut grid: ResMut<bevy_glacial::prelude::GroundGrid>,
    glacial_counters: Option<Res<bevy_glacial::prelude::GlacialGridCounters>>,
    mut counters: Option<ResMut<crate::viewport::diagnostics::performance::RendererCounters>>,
) {
    let desired_ground_y = match toggles.ground_grid_origin {
        GroundGridOrigin::LoadedScene => extent.geometry_ground_y(),
        GroundGridOrigin::WorldOrigin => Some(0.0),
    };

    let camera_distance = cameras
        .single()
        .map(|camera| camera.distance)
        .unwrap_or(0.0);
    let desired_radius = (extent.diag().max(camera_distance) * 2.5).max(
        bevy_glacial::prelude::LEVEL_HALF
            .last()
            .copied()
            .unwrap_or(640.0),
    );

    let prev_ground_y = grid.ground_y;
    let prev_radius = grid.coverage_radius;
    let prev_vis = grid.visible;

    let ground_y_changed = GroundGridDecisionHelper::optional_field_changed(
        prev_ground_y,
        desired_ground_y,
        GroundGridDecisionHelper::DEFAULT_TOLERANCE,
    );
    let coverage_radius_changed = GroundGridDecisionHelper::needs_radius_update(
        prev_radius,
        desired_radius,
        GroundGridDecisionHelper::DEFAULT_TOLERANCE,
    );
    let visibility_changed = prev_vis != toggles.renderer.grid;

    if ground_y_changed {
        grid.ground_y = desired_ground_y;
    }
    if coverage_radius_changed {
        grid.coverage_radius = desired_radius;
    }
    if visibility_changed {
        grid.visible = toggles.renderer.grid;
    }

    if let Some(ref mut c) = counters {
        c.grid_sync_calls += 1;
        if ground_y_changed {
            c.grid_host_writes += 1;
            c.grid_ground_y_writes += 1;
            c.grid_value_changes += 1;
        }
        if coverage_radius_changed {
            c.grid_host_writes += 1;
            c.grid_coverage_radius_writes += 1;
            c.grid_value_changes += 1;
        }
        if visibility_changed {
            c.grid_host_writes += 1;
            c.grid_visible_writes += 1;
            c.grid_value_changes += 1;
        }
        if grid.is_changed() {
            c.grid_changed_observations += 1;
        }

        if let Some(ref gc) = glacial_counters {
            c.grid_update_alpha_calls = gc.alpha_rebuild_calls;
            c.grid_lines_rebuilt = gc.lines_rebuilt;
            c.grid_dots_rebuilt = gc.dots_rebuilt;
            c.grid_structural_rebuilds = gc.alpha_rebuild_calls;
            c.grid_vertices_generated = gc.vertices_generated;
            c.grid_indices_generated = gc.indices_generated;
        }
    }
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
                    capture_original_shadow_settings,
                    apply_shadow_toggle,
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

/// Captured authored shadow policy for one light. Global renderer shadow
/// control temporarily disables shadows without losing authored per-light
/// choices that must be restored when shadows are enabled again.
#[derive(Component, Debug, Copy, Clone)]
pub struct OriginalShadowEnabled(pub bool);

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

/// Latches each light's authored shadow policy exactly once.
fn capture_original_shadow_settings(
    mut cmds: Commands,
    dir: Query<
        (Entity, &DirectionalLight),
        (Added<DirectionalLight>, Without<OriginalShadowEnabled>),
    >,
    pt: Query<(Entity, &PointLight), (Added<PointLight>, Without<OriginalShadowEnabled>)>,
    sp: Query<(Entity, &SpotLight), (Added<SpotLight>, Without<OriginalShadowEnabled>)>,
) {
    for (entity, light) in &dir {
        cmds.entity(entity)
            .insert(OriginalShadowEnabled(light.shadow_maps_enabled));
    }
    for (entity, light) in &pt {
        cmds.entity(entity)
            .insert(OriginalShadowEnabled(light.shadow_maps_enabled));
    }
    for (entity, light) in &sp {
        cmds.entity(entity)
            .insert(OriginalShadowEnabled(light.shadow_maps_enabled));
    }
}

/// Applies the global shadow option while preserving authored per-light state.
fn apply_shadow_toggle(
    toggles: Res<DisplayToggles>,
    mut dir: Query<(&mut DirectionalLight, &OriginalShadowEnabled)>,
    mut pt: Query<(&mut PointLight, &OriginalShadowEnabled)>,
    mut sp: Query<(&mut SpotLight, &OriginalShadowEnabled)>,
) {
    for (mut light, authored) in &mut dir {
        let desired = toggles.renderer.shadows && authored.0;
        if light.shadow_maps_enabled != desired {
            light.shadow_maps_enabled = desired;
        }
    }
    for (mut light, authored) in &mut pt {
        let desired = toggles.renderer.shadows && authored.0;
        if light.shadow_maps_enabled != desired {
            light.shadow_maps_enabled = desired;
        }
    }
    for (mut light, authored) in &mut sp {
        let desired = toggles.renderer.shadows && authored.0;
        if light.shadow_maps_enabled != desired {
            light.shadow_maps_enabled = desired;
        }
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
    let wireframe = toggles.renderer.render_mode == RenderMode::Wireframe;
    if cfg.global != wireframe {
        cfg.global = wireframe;
    }
}

/// Persistent overlay state, mutated by the Overlays panel + keyboard.
#[derive(Resource, Debug, Clone)]
pub struct DisplayToggles {
    /// Authoritative renderer options applied by the viewport systems.
    pub renderer: RendererConfiguration,
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
            renderer: RendererConfiguration::default(),
            ground_grid_origin: GroundGridOrigin::LoadedScene,
            show_world_axes: false,
            show_prim_markers: false,
            prim_marker_bias: 1.0,
            show_skeleton: false,
            show_physics: false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_glacial::prelude::GroundGrid;

    #[test]
    fn grid_visibility_reads_the_authoritative_renderer_configuration() {
        let mut app = App::new();
        app.insert_resource(DisplayToggles::default())
            .insert_resource(GroundGrid {
                visible: true,
                ..default()
            })
            .add_systems(Update, sync_ground_grid_visibility);

        app.world_mut()
            .resource_mut::<DisplayToggles>()
            .renderer
            .grid = false;
        app.update();

        assert!(!app.world().resource::<GroundGrid>().visible);
    }

    #[test]
    fn shadows_disable_globally_and_restore_each_authored_setting() {
        let mut app = App::new();
        let authored_on = app
            .world_mut()
            .spawn(DirectionalLight {
                shadow_maps_enabled: true,
                ..default()
            })
            .id();
        let authored_off = app
            .world_mut()
            .spawn(DirectionalLight {
                shadow_maps_enabled: false,
                ..default()
            })
            .id();
        app.insert_resource(DisplayToggles::default()).add_systems(
            Update,
            (capture_original_shadow_settings, apply_shadow_toggle).chain(),
        );

        app.update();
        app.update();
        assert!(
            app.world()
                .get::<DirectionalLight>(authored_on)
                .unwrap()
                .shadow_maps_enabled
        );
        assert!(
            !app.world()
                .get::<DirectionalLight>(authored_off)
                .unwrap()
                .shadow_maps_enabled
        );

        app.world_mut()
            .resource_mut::<DisplayToggles>()
            .renderer
            .shadows = false;
        app.update();
        assert!(
            !app.world()
                .get::<DirectionalLight>(authored_on)
                .unwrap()
                .shadow_maps_enabled
        );
        assert!(
            !app.world()
                .get::<DirectionalLight>(authored_off)
                .unwrap()
                .shadow_maps_enabled
        );

        app.world_mut()
            .resource_mut::<DisplayToggles>()
            .renderer
            .shadows = true;
        app.update();
        assert!(
            app.world()
                .get::<DirectionalLight>(authored_on)
                .unwrap()
                .shadow_maps_enabled
        );
        assert!(
            !app.world()
                .get::<DirectionalLight>(authored_off)
                .unwrap()
                .shadow_maps_enabled
        );
    }

    #[test]
    fn render_mode_round_trip_updates_bevy_wireframe_without_touching_edges() {
        let mut app = App::new();
        app.insert_resource(DisplayToggles {
            renderer: RendererConfiguration {
                edges: true,
                ..default()
            },
            ..default()
        })
        .insert_resource(bevy::pbr::wireframe::WireframeConfig {
            global: false,
            ..default()
        })
        .add_systems(Update, apply_wireframe_toggle);

        app.world_mut()
            .resource_mut::<DisplayToggles>()
            .renderer
            .render_mode = RenderMode::Wireframe;
        app.update();
        assert!(
            app.world()
                .resource::<bevy::pbr::wireframe::WireframeConfig>()
                .global
        );
        assert!(app.world().resource::<DisplayToggles>().renderer.edges);

        app.world_mut()
            .resource_mut::<DisplayToggles>()
            .renderer
            .render_mode = RenderMode::Shaded;
        app.update();
        assert!(
            !app.world()
                .resource::<bevy::pbr::wireframe::WireframeConfig>()
                .global
        );
        assert!(app.world().resource::<DisplayToggles>().renderer.edges);
    }
}
