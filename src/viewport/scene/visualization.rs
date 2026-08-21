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

use std::collections::{HashMap, HashSet};

use bevy::asset::{RenderAssetUsages, prelude::AssetChanged};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use usd_bevy::UsdPrimRef;
use viewport_protocol::{GroundGridOrigin, RenderMode, RendererConfiguration};

use super::HistoricalGhostState;
use super::{draw_semantic_diff, hydrate_historical_ghosts};
use crate::viewport::camera::ArcballCamera;
use crate::viewport::diagnostics::performance::GroundGridDecisionHelper;

pub struct OverlaysPlugin;

/// Cached edge geometry and its shared presentation material.
#[derive(Resource, Debug, Default)]
struct EdgeOverlayCache {
    meshes: HashMap<AssetId<Mesh>, Handle<Mesh>>,
    last_enabled: Option<bool>,
}

/// Observable proof that the independent edge pass is enabled and doing work.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct EdgeOverlayStats {
    pub enabled: bool,
    pub cached_meshes: u64,
    pub mesh_builds: u64,
}

#[derive(Resource, Debug, Clone)]
struct EdgeOverlayMaterial(Handle<StandardMaterial>);

/// Marks a child entity as the cached edge pass for one USD mesh entity.
#[derive(Component, Debug)]
struct EdgeOverlay {
    source_mesh: AssetId<Mesh>,
}

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
            .init_resource::<EdgeOverlayCache>()
            .init_resource::<EdgeOverlayStats>()
            .add_systems(Startup, init_edge_overlay_material)
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
                    sync_edge_overlays,
                    sync_ground_grid_visibility,
                    hydrate_historical_ghosts,
                    draw_semantic_diff,
                )
                    .chain()
                    .before(bevy_glacial::prelude::build_grid_meshes),
            );
    }
}

fn init_edge_overlay_material(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    existing: Option<Res<EdgeOverlayMaterial>>,
) {
    if existing.is_some() {
        return;
    }

    commands.insert_resource(EdgeOverlayMaterial(materials.add(StandardMaterial {
        base_color: Color::srgba(0.12, 0.80, 1.0, 0.9),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        depth_bias: 1.0,
        ..default()
    })));
}

/// Synchronizes the independent edge pass only on a renderer-config or mesh
/// change. The source USD entity owns the transform; the child owns only the
/// cached line-list geometry and presentation material.
fn sync_edge_overlays(
    toggles: Res<DisplayToggles>,
    mut mesh_assets: ResMut<Assets<Mesh>>,
    material: Option<Res<EdgeOverlayMaterial>>,
    mut cache: ResMut<EdgeOverlayCache>,
    mut stats: ResMut<EdgeOverlayStats>,
    mut counters: Option<ResMut<crate::viewport::diagnostics::performance::RendererCounters>>,
    mut commands: Commands,
    sources: Query<(Entity, &Mesh3d, Option<&Children>), With<UsdPrimRef>>,
    changed_sources: Query<
        (Entity, &Mesh3d, Option<&Children>),
        (
            With<UsdPrimRef>,
            Or<(Added<Mesh3d>, Changed<Mesh3d>, AssetChanged<Mesh3d>)>,
        ),
    >,
    source_children: Query<&Children, With<UsdPrimRef>>,
    mut overlays: Query<
        (Entity, &mut EdgeOverlay, &mut Mesh3d, &mut Visibility),
        Without<UsdPrimRef>,
    >,
    mut removed_meshes: RemovedComponents<Mesh3d>,
) {
    let Some(material) = material else {
        return;
    };

    stats.enabled = toggles.renderer.edges;

    for source in removed_meshes.read() {
        let Ok(children) = source_children.get(source) else {
            continue;
        };
        for child in children {
            if overlays.get(*child).is_ok() {
                commands.entity(*child).despawn();
            }
        }
    }

    let edge_toggle_changed = cache.last_enabled != Some(toggles.renderer.edges);
    cache.last_enabled = Some(toggles.renderer.edges);

    if edge_toggle_changed {
        for (entity, mesh, children) in &sources {
            sync_one_edge_overlay(
                &mut commands,
                entity,
                mesh,
                children,
                toggles.renderer.edges,
                &mut mesh_assets,
                &material.0,
                &mut cache,
                &mut stats,
                &mut overlays,
            );
        }
    } else {
        for (entity, mesh, children) in &changed_sources {
            cache.meshes.remove(&mesh.0.id());
            sync_one_edge_overlay(
                &mut commands,
                entity,
                mesh,
                children,
                toggles.renderer.edges,
                &mut mesh_assets,
                &material.0,
                &mut cache,
                &mut stats,
                &mut overlays,
            );
        }
    }

    stats.cached_meshes = cache.meshes.len() as u64;
    if let Some(ref mut counters) = counters
        && counters.configuration_edges_enabled != stats.enabled
    {
        counters.configuration_edges_enabled = stats.enabled;
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_one_edge_overlay(
    commands: &mut Commands,
    source: Entity,
    mesh: &Mesh3d,
    children: Option<&Children>,
    enabled: bool,
    mesh_assets: &mut Assets<Mesh>,
    material: &Handle<StandardMaterial>,
    cache: &mut EdgeOverlayCache,
    stats: &mut EdgeOverlayStats,
    overlays: &mut Query<
        (Entity, &mut EdgeOverlay, &mut Mesh3d, &mut Visibility),
        Without<UsdPrimRef>,
    >,
) {
    let source_mesh_id = mesh.0.id();
    let existing_child =
        children.and_then(|children| children.iter().find(|child| overlays.get(*child).is_ok()));

    if mesh_assets.get(source_mesh_id).is_none() {
        if let Some(child) = existing_child
            && let Ok((_, _, _, mut visibility)) = overlays.get_mut(child)
        {
            set_edge_visibility(&mut visibility, false);
        }
        return;
    }

    let edge_handle = if enabled {
        edge_mesh_handle(source_mesh_id, mesh_assets, cache, stats)
    } else {
        None
    };

    if let Some(child) = existing_child {
        let Ok((_, mut overlay, mut edge_mesh, mut visibility)) = overlays.get_mut(child) else {
            return;
        };

        overlay.source_mesh = source_mesh_id;
        if let Some(edge_handle) = edge_handle {
            edge_mesh.0 = edge_handle;
            set_edge_visibility(&mut visibility, true);
        } else {
            set_edge_visibility(&mut visibility, false);
        }
        return;
    }

    let Some(edge_handle) = edge_handle else {
        return;
    };

    commands.entity(source).with_children(|parent| {
        parent.spawn((
            EdgeOverlay {
                source_mesh: source_mesh_id,
            },
            Mesh3d(edge_handle),
            MeshMaterial3d(material.clone()),
            bevy::pbr::wireframe::NoWireframe,
            if enabled {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
        ));
    });
}

fn set_edge_visibility(visibility: &mut Visibility, visible: bool) {
    let desired = if visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if *visibility != desired {
        *visibility = desired;
    }
}

fn edge_mesh_handle(
    source_mesh_id: AssetId<Mesh>,
    mesh_assets: &mut Assets<Mesh>,
    cache: &mut EdgeOverlayCache,
    stats: &mut EdgeOverlayStats,
) -> Option<Handle<Mesh>> {
    if let Some(handle) = cache.meshes.get(&source_mesh_id) {
        return Some(handle.clone());
    }

    let edge_mesh = {
        let source_mesh = mesh_assets.get(source_mesh_id)?;
        build_edge_mesh(source_mesh)?
    };
    let handle = mesh_assets.add(edge_mesh);
    cache.meshes.insert(source_mesh_id, handle.clone());
    stats.mesh_builds += 1;
    Some(handle)
}

fn build_edge_mesh(source: &Mesh) -> Option<Mesh> {
    let VertexAttributeValues::Float32x3(positions) = source.attribute(Mesh::ATTRIBUTE_POSITION)?
    else {
        return None;
    };

    let indices: Vec<u32> = match source.indices() {
        Some(Indices::U16(indices)) => indices.iter().map(|index| u32::from(*index)).collect(),
        Some(Indices::U32(indices)) => indices.clone(),
        None => (0..positions.len() as u32).collect(),
    };

    let mut edge_indices = Vec::with_capacity(indices.len() * 2);
    let mut seen = HashSet::with_capacity(indices.len());
    match source.primitive_topology() {
        PrimitiveTopology::TriangleList => {
            for triangle in indices.chunks_exact(3) {
                add_edge(&mut seen, &mut edge_indices, triangle[0], triangle[1]);
                add_edge(&mut seen, &mut edge_indices, triangle[1], triangle[2]);
                add_edge(&mut seen, &mut edge_indices, triangle[2], triangle[0]);
            }
        }
        PrimitiveTopology::TriangleStrip => {
            for triangle in indices.windows(3) {
                add_edge(&mut seen, &mut edge_indices, triangle[0], triangle[1]);
                add_edge(&mut seen, &mut edge_indices, triangle[1], triangle[2]);
                add_edge(&mut seen, &mut edge_indices, triangle[2], triangle[0]);
            }
        }
        _ => return None,
    }

    if edge_indices.is_empty() {
        return None;
    }

    let mut edge_mesh = Mesh::new(
        PrimitiveTopology::LineList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    edge_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.clone());
    edge_mesh.insert_indices(Indices::U32(edge_indices));
    Some(edge_mesh)
}

fn add_edge(seen: &mut HashSet<(u32, u32)>, output: &mut Vec<u32>, a: u32, b: u32) {
    if a == b {
        return;
    }
    let edge = if a < b { (a, b) } else { (b, a) };
    if seen.insert(edge) {
        output.extend_from_slice(&[edge.0, edge.1]);
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

    fn triangle_mesh() -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
        mesh
    }

    fn edge_child(world: &World, source: Entity) -> Option<Entity> {
        world
            .get::<Children>(source)?
            .iter()
            .find(|child| world.get::<EdgeOverlay>(*child).is_some())
    }

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

    #[test]
    fn edge_overlay_is_independent_from_wireframe_for_all_four_combinations() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(DisplayToggles::default())
            .init_resource::<EdgeOverlayCache>()
            .init_resource::<EdgeOverlayStats>()
            .insert_resource(bevy::pbr::wireframe::WireframeConfig::default())
            .add_systems(Update, (apply_wireframe_toggle, sync_edge_overlays).chain());
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        app.insert_resource(EdgeOverlayMaterial(material));
        let source_mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(triangle_mesh());
        let source = app
            .world_mut()
            .spawn((UsdPrimRef::new("/Triangle"), Mesh3d(source_mesh)))
            .id();

        // Shaded edge off.
        app.update();
        assert!(!app.world().resource::<EdgeOverlayStats>().enabled);
        assert_eq!(edge_child(app.world(), source), None);
        assert!(
            !app.world()
                .resource::<bevy::pbr::wireframe::WireframeConfig>()
                .global
        );

        // Shaded edge on.
        app.world_mut()
            .resource_mut::<DisplayToggles>()
            .renderer
            .edges = true;
        app.update();
        let child = edge_child(app.world(), source).expect("edge pass should create a child mesh");
        assert_eq!(
            app.world().get::<Visibility>(child),
            Some(&Visibility::Inherited)
        );
        assert!(
            app.world()
                .get::<bevy::pbr::wireframe::NoWireframe>(child)
                .is_some()
        );
        assert!(
            !app.world()
                .resource::<bevy::pbr::wireframe::WireframeConfig>()
                .global
        );
        let edge_mesh = app
            .world()
            .resource::<Assets<Mesh>>()
            .get(app.world().get::<Mesh3d>(child).unwrap().0.id())
            .expect("edge child must reference cached line geometry");
        assert_eq!(edge_mesh.primitive_topology(), PrimitiveTopology::LineList);
        assert_eq!(edge_mesh.indices().unwrap().len(), 6);
        assert_eq!(app.world().resource::<EdgeOverlayStats>().mesh_builds, 1);

        // Wireframe edge off.
        app.world_mut().resource_mut::<DisplayToggles>().renderer = RendererConfiguration {
            edges: false,
            render_mode: RenderMode::Wireframe,
            ..Default::default()
        };
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(child),
            Some(&Visibility::Hidden)
        );
        assert!(
            app.world()
                .resource::<bevy::pbr::wireframe::WireframeConfig>()
                .global
        );

        // Wireframe edge on.
        app.world_mut()
            .resource_mut::<DisplayToggles>()
            .renderer
            .edges = true;
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(child),
            Some(&Visibility::Inherited)
        );
        assert!(
            app.world()
                .resource::<bevy::pbr::wireframe::WireframeConfig>()
                .global
        );
        assert_eq!(app.world().resource::<EdgeOverlayStats>().mesh_builds, 1);
    }

    #[test]
    fn edge_mesh_deduplicates_shared_triangle_edges_and_rejects_points() {
        let mut quad = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD,
        );
        quad.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
        );
        quad.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
        let edges = build_edge_mesh(&quad).expect("triangle topology should produce edges");
        assert_eq!(edges.indices().unwrap().len(), 10);

        let points = Mesh::new(PrimitiveTopology::PointList, RenderAssetUsages::MAIN_WORLD);
        assert!(build_edge_mesh(&points).is_none());
    }
}
