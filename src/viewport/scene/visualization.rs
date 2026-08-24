//! Authoritative renderer presentation systems.
//!
//! Grid synchronization, light policy, wireframe mode, and edge geometry are
//! kept in separate modules so each responsibility stays below the source
//! size gate and can be tested without a monolithic overlay implementation.

use bevy::prelude::*;
use viewport_protocol::{GroundGridOrigin, RendererConfiguration};

use super::HistoricalGhostState;
use super::selection_color::{
    SelectionColorOverrideState, init_selection_color_material, sync_selection_color_overrides,
};
use super::selection_hover::{HoverPickStats, HoveredTarget, update_hover_target};
use super::{
    SectionBoxState, capture_section_box_gizmo_transform, draw_section_box,
    sync_section_box_clipping, sync_section_box_gizmo_target, sync_section_box_state,
};
use super::{draw_semantic_diff, hydrate_historical_ghosts};
use crate::viewport::camera::ArcballCamera;

#[path = "visualization_edge.rs"]
mod edge;
#[path = "visualization_edge_mesh.rs"]
mod edge_mesh;
#[path = "visualization_environment.rs"]
mod environment;
#[path = "visualization_render_mode.rs"]
mod render_mode;
#[path = "visualization_shadows.rs"]
mod shadows;

use edge::{EdgeOverlayCache, init_edge_overlay_material, sync_edge_overlays};
pub(super) use environment::{
    sync_background_color, sync_fallback_surface_color, sync_ground_grid_to_scene,
    sync_ground_grid_visibility,
};
use render_mode::{apply_render_mode, apply_wireframe_toggle, init_uniform_render_material};
use shadows::{ShadowProjectionState, apply_shadow_toggle, capture_original_shadow_settings};

pub(crate) use edge::{EdgeOverlay, EdgeOverlayStats};

pub struct OverlaysPlugin;

impl Plugin for OverlaysPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DisplayToggles>()
            .init_resource::<SceneExtent>()
            .init_resource::<HistoricalGhostState>()
            .init_resource::<EdgeOverlayCache>()
            .init_resource::<EdgeOverlayStats>()
            .init_resource::<SelectionColorOverrideState>()
            .init_resource::<HoveredTarget>()
            .init_resource::<HoverPickStats>()
            .init_resource::<SectionBoxState>()
            .init_resource::<super::section_box_clipping::SectionClipProjectionState>()
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
                    capture_section_box_gizmo_transform,
                    compute_extent,
                    sync_section_box_state,
                    sync_section_box_gizmo_target,
                    draw_section_box,
                )
                    .chain()
                    .after(crate::viewport::api::ViewportBridgeSet::ApplyCommands)
                    .after(crate::viewport::camera::ArcballCameraSet::ApplyInput)
                    .before(sync_ground_grid_to_scene)
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
                    update_hover_target,
                    sync_selection_color_overrides,
                    sync_section_box_clipping,
                    apply_wireframe_toggle,
                    sync_edge_overlays,
                    sync_ground_grid_visibility,
                    hydrate_historical_ghosts,
                    draw_semantic_diff,
                )
                    .chain()
                    .after(crate::viewport::api::ViewportBridgeSet::ApplyCommands)
                    .after(crate::viewport::camera::ArcballCameraSet::ApplyInput)
                    .before(bevy_glacial::prelude::build_grid_meshes),
            );
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

pub use super::extent::SceneExtent;
use super::extent::compute_extent;

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
#[path = "visualization_tests.rs"]
mod tests;
