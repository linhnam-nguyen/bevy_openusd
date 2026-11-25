//! Authoritative renderer presentation systems.
//!
//! Grid synchronization, light policy, wireframe mode, and edge geometry are
//! kept in separate modules so each responsibility stays below the source
//! size gate and can be tested without a monolithic overlay implementation.

use bevy::prelude::*;
use viewport_protocol::{
    GroundGridOrigin, MAX_GIZMO_SIZE_LEVEL, MIN_GIZMO_SIZE_LEVEL, RendererConfiguration,
};

use crate::api::ViewerSettingsState;
use crate::viewport::navigation::camera::ArcballCamera;
use crate::viewport::rendering::scene::SelectionPresentationPolicy;
use crate::viewport::navigation::section_box::{
    SectionBoxState, SectionClipDiagnostics, SectionClipProjectionState,
    capture_section_box_gizmo_transform, draw_section_box, register_embedded_shaders,
    sync_section_box_clipping, sync_section_box_gizmo_target, sync_section_box_state,
};
use crate::viewport::navigation::selection::{
    CoarseSelectionProxyState, HoverPickStats, HoveredTarget, SelectedRenderableProjection,
    SelectionColorOverrideState, draw_coarse_selection_proxy, init_selection_color_material,
    register_selection_projection_observers, sync_coarse_selection_proxy,
    sync_selected_renderable_projection, sync_selection_color_overrides, update_hover_target,
};

mod edge;
mod environment;
mod render_mode;
mod shadows;

use edge::{EdgeOverlayCache, init_edge_overlay_material, sync_edge_overlays};
pub(super) use environment::{
    sync_background_color, sync_fallback_surface_color, sync_ground_grid_to_scene,
    sync_ground_grid_visibility,
};
pub(crate) use render_mode::{OriginalRenderMaterial, UniformRenderMaterial};
use render_mode::{apply_render_mode, apply_wireframe_toggle, init_uniform_render_material};
use shadows::{ShadowProjectionState, apply_shadow_toggle, capture_original_shadow_settings};

pub(crate) use edge::{EdgeOverlay, EdgeOverlayStats};

/// Neutral visual compatibility flag updated by optional product services
/// (e.g. classification color or semantic diff overlays) so generic presentation
/// does not import BIM or semantic modules.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PresentationVisualCompatibility {
    pub(crate) block_gpu_instancing: bool,
}

/// Neutral ordered presentation overlay phases so optional product overlays
/// (e.g. classification color or historical ghosts/semantic diff) slot into the
/// exact cross-system sequence without reverse imports from generic
/// `viewport::presentation` into product modules.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum PresentationOverlaySet {
    /// Environment, light/shadow, and base render-mode systems.
    BaseRenderMode,
    /// Optional product styling overlays applied after base render mode and
    /// before hover/selection/clip/wireframe/edge overlays.
    ProductStyle,
    /// Generic hover, selection color, section-box clipping, wireframe, edge,
    /// and ground-grid visibility overlays.
    SelectionAndOverlays,
    /// Optional post-overlay product systems (e.g. historical ghosts, diff).
    PostOverlay,
}

pub struct OverlaysPlugin;

impl Plugin for OverlaysPlugin {
    fn build(&self, app: &mut App) {
        register_embedded_shaders(app);
        register_selection_projection_observers(app);
        app.init_resource::<DisplayToggles>()
            .init_resource::<PresentationVisualCompatibility>()
            .init_resource::<SceneExtent>()
            .init_resource::<EdgeOverlayCache>()
            .init_resource::<EdgeOverlayStats>()
            .init_resource::<SelectionColorOverrideState>()
            .init_resource::<SelectionPresentationPolicy>()
            .init_resource::<CoarseSelectionProxyState>()
            .init_resource::<HoveredTarget>()
            .init_resource::<crate::viewport::navigation::selection::PointClick>()
            .init_resource::<HoverPickStats>()
            .init_resource::<SectionBoxState>()
            .init_resource::<SelectedRenderableProjection>()
            .init_resource::<SectionClipProjectionState>()
            .init_resource::<SectionClipDiagnostics>()
            .init_resource::<render_mode::RenderModeProjectionState>()
            .init_resource::<ShadowProjectionState>()
            .add_systems(
                Startup,
                (
                    init_edge_overlay_material,
                    init_uniform_render_material,
                    init_selection_color_material,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    sync_point_instancing_policy,
                    sync_selected_renderable_projection,
                    sync_coarse_selection_proxy,
                    sync_gizmo_size,
                    capture_section_box_gizmo_transform,
                    compute_extent,
                    sync_section_box_state,
                    sync_section_box_gizmo_target,
                    draw_section_box,
                    draw_coarse_selection_proxy,
                )
                    .chain()
                    .after(crate::api::ViewportBridgeSet::ApplyCommands)
                    .after(crate::viewport::navigation::camera::ArcballCameraSet::ApplyInput)
                    .before(sync_ground_grid_to_scene)
                    .before(bevy_glacial::prelude::build_grid_meshes),
            )
            .configure_sets(
                Update,
                (
                    PresentationOverlaySet::BaseRenderMode,
                    PresentationOverlaySet::ProductStyle,
                    PresentationOverlaySet::SelectionAndOverlays,
                    PresentationOverlaySet::PostOverlay,
                )
                    .chain()
                    .after(crate::api::ViewportBridgeSet::ApplyCommands)
                    .after(crate::viewport::navigation::camera::ArcballCameraSet::ApplyInput)
                    .after(sync_selected_renderable_projection)
                    .after(sync_section_box_state)
                    .before(bevy_glacial::prelude::build_grid_meshes),
            )
            .add_systems(
                Update,
                (
                    sync_ground_grid_to_scene,
                    sync_background_color,
                    sync_fallback_surface_color,
                    sync_shadow_cascade_distance,
                    capture_original_light_levels,
                    capture_original_shadow_settings,
                    apply_shadow_toggle,
                    apply_light_intensity_scale,
                    apply_render_mode,
                )
                    .chain()
                    .in_set(PresentationOverlaySet::BaseRenderMode),
            )
            .add_systems(
                Update,
                (
                    (
                        update_hover_target,
                        crate::viewport::navigation::selection::select_point_click,
                    )
                        .chain(),
                    sync_selection_color_overrides,
                    sync_section_box_clipping,
                    apply_wireframe_toggle,
                    sync_edge_overlays,
                    sync_ground_grid_visibility,
                )
                    .chain()
                    .in_set(PresentationOverlaySet::SelectionAndOverlays),
            );
    }

    fn finish(&self, app: &mut App) {
        crate::viewport::rendering::gpu_limits::install_render_guards(app);
    }
}

pub(crate) fn sync_point_instancing_policy(world: &mut World, mut last: Local<Option<(u64, u64, bool)>>) {
    if !world.contains_resource::<usd_bevy::route::gpu_instancing::UsdGpuInstancing>() {
        return;
    }
    let renderer = &world.resource::<DisplayToggles>().renderer;
    let compatible = renderer.render_mode == viewport_protocol::RenderMode::Shaded
        && !renderer.edges
        && world
            .get_resource::<PresentationVisualCompatibility>()
            .is_none_or(|compat| !compat.block_gpu_instancing)
        && !world
            .resource::<ViewerSettingsState>()
            .section_box_enabled();
    let selection = world.resource::<crate::viewport::navigation::selection::SelectedTargets>();
    let index = world.resource::<crate::viewport::rendering::scene::SceneAnchorIndex>();
    let revision = (selection.revision(), index.revision(), compatible);
    if *last == Some(revision) {
        return;
    }
    *last = Some(revision);
    let mut points = std::collections::BTreeMap::<_, Vec<_>>::new();
    for anchor in &selection.0.targets {
        if let Some(key) = index.point_presentation_key(anchor) {
            points
                .entry(anchor.prim_path.clone())
                .or_default()
                .push(key);
        }
    }
    for group in points.values_mut() {
        group.sort_unstable();
    }
    usd_bevy::live::sync_point_instancing_presentation(world, compatible, points);
}

/// Applies the user's Selection-panel gizmo size level to both ordinary gizmo
/// primitives and the Section Box face draggers. The level is log-spaced from
/// 2 (the current size) through 10 (ten times the current size).
fn sync_gizmo_size(
    settings: Res<ViewerSettingsState>,
    mut gizmo_options: ResMut<bevy_glacial::prelude::GizmoOptions>,
) {
    let level = settings
        .selection()
        .gizmo_size_level
        .clamp(MIN_GIZMO_SIZE_LEVEL, MAX_GIZMO_SIZE_LEVEL);
    let exponent = (level - MIN_GIZMO_SIZE_LEVEL) as f32
        / (MAX_GIZMO_SIZE_LEVEL - MIN_GIZMO_SIZE_LEVEL) as f32;
    let scale = 10.0_f32.powf(exponent);

    if (gizmo_options.gizmo_size_scale - scale).abs() > f32::EPSILON {
        gizmo_options.gizmo_size_scale = scale;
    }
}

#[derive(Component, Debug, Copy, Clone)]
pub struct OriginalIlluminance(pub f32);

#[derive(Component, Debug, Copy, Clone)]
pub struct OriginalLightIntensity(pub f32);

#[derive(Component, Debug, Copy, Clone)]
pub struct OriginalShadowEnabled(pub bool);

fn capture_original_light_levels(
    mut cmds: Commands,
    dir: Query<
        (Entity, &DirectionalLight),
        (Added<DirectionalLight>, Without<OriginalIlluminance>),
    >,
    pt: Query<(Entity, &PointLight), (Added<PointLight>, Without<OriginalLightIntensity>)>,
    sp: Query<(Entity, &SpotLight), (Added<SpotLight>, Without<OriginalLightIntensity>)>,
) {
    for (entity, light) in &dir {
        cmds.entity(entity)
            .insert(OriginalIlluminance(light.illuminance));
    }
    for (entity, light) in &pt {
        cmds.entity(entity)
            .insert(OriginalLightIntensity(light.intensity));
    }
    for (entity, light) in &sp {
        cmds.entity(entity)
            .insert(OriginalLightIntensity(light.intensity));
    }
}

fn apply_light_intensity_scale(
    toggles: Res<DisplayToggles>,
    mut dir: Query<(&mut DirectionalLight, &OriginalIlluminance)>,
    mut pt: Query<(&mut PointLight, &OriginalLightIntensity)>,
    mut sp: Query<(&mut SpotLight, &OriginalLightIntensity)>,
) {
    let scale = toggles.light_intensity_scale;
    for (mut light, authored) in &mut dir {
        light.illuminance = authored.0 * scale;
    }
    for (mut light, authored) in &mut pt {
        light.intensity = authored.0 * scale;
    }
    for (mut light, authored) in &mut sp {
        light.intensity = authored.0 * scale;
    }
}

#[derive(Resource, Debug, Clone)]
pub struct DisplayToggles {
    pub renderer: RendererConfiguration,
    pub ground_grid_origin: GroundGridOrigin,
    pub show_world_axes: bool,
    pub show_prim_markers: bool,
    pub prim_marker_bias: f32,
    pub show_skeleton: bool,
    pub show_physics: bool,
    pub show_colliders: bool,
    pub light_intensity_scale: f32,
}

impl Default for DisplayToggles {
    fn default() -> Self {
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

pub use crate::viewport::rendering::scene::extent::SceneExtent;
use crate::viewport::rendering::scene::extent::compute_extent;

fn sync_shadow_cascade_distance(
    extent: Res<SceneExtent>,
    cameras: Query<&ArcballCamera>,
    mut lights: Query<&mut bevy::light::CascadeShadowConfig, With<DirectionalLight>>,
) {
    let Ok(camera) = cameras.single() else {
        return;
    };
    let (min, max) = if extent.geometry_count > 0 {
        (extent.geometry_min, extent.geometry_max)
    } else if extent.count > 0 {
        (extent.min, extent.max)
    } else {
        return;
    };
    let focus_to_center = (min * 0.5 + max * 0.5).distance(camera.focus);
    let radius = (max - min).length() * 0.5;
    let far = (camera.distance + focus_to_center + radius) * 1.1;
    if !far.is_finite() || min.cmpgt(max).any() {
        return;
    }
    let desired = far.clamp(150.0, 10_000.0);
    for mut config in &mut lights {
        let Some(current) = config
            .bounds
            .last()
            .copied()
            .filter(|value| value.is_finite())
        else {
            continue;
        };
        let first = config.bounds.first().copied().unwrap_or(current);
        let cascade_count = config.bounds.len();
        if (current - desired).abs() > current.max(1.0) * 0.05
            && config.minimum_distance >= 0.0
            && config.minimum_distance < desired
            && (0.0..1.0).contains(&config.overlap_proportion)
            && (cascade_count == 1 || (config.minimum_distance < first && first < desired))
        {
            *config = bevy::light::CascadeShadowConfigBuilder {
                num_cascades: cascade_count,
                first_cascade_far_bound: first,
                minimum_distance: config.minimum_distance,
                overlap_proportion: config.overlap_proportion,
                maximum_distance: desired,
            }
            .into();
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/rendering/presentation/tests.rs"]
mod tests;
