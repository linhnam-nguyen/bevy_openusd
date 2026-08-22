//! Renderer-owned environment synchronization for the native viewport.
//!
//! The viewport bridge owns the applied protocol settings; these systems only
//! project them into the existing Bevy/Glacial presentation resources.

use bevy::prelude::*;
use viewport_protocol::{ColorRgb8, GroundGridOrigin};

use super::{DisplayToggles, SceneExtent};
use crate::viewport::api::ViewerSettingsState;
use crate::viewport::camera::ArcballCamera;
use crate::viewport::diagnostics::performance::GroundGridDecisionHelper;

pub(crate) fn color_from_rgb8(color: ColorRgb8) -> Color {
    Color::srgb(
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
    )
}

/// Keeps Glacial's ground-grid visibility aligned with the renderer state.
pub(crate) fn sync_ground_grid_visibility(
    toggles: Res<DisplayToggles>,
    mut grid: ResMut<bevy_glacial::prelude::GroundGrid>,
) {
    if grid.visible != toggles.renderer.grid {
        grid.visible = toggles.renderer.grid;
    }
}

/// Projects visibility, color, origin, and bounded scene coverage into the
/// one Glacial `GroundGrid` resource. Camera motion changes transforms and
/// coverage only when the tolerance says the values actually changed.
pub(crate) fn sync_ground_grid_to_scene(
    extent: Res<SceneExtent>,
    cameras: Query<&ArcballCamera>,
    toggles: Res<DisplayToggles>,
    viewer_settings: Res<ViewerSettingsState>,
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
    let desired_color = color_from_rgb8(viewer_settings.environment().grid_color);

    let ground_y_changed = GroundGridDecisionHelper::optional_field_changed(
        grid.ground_y,
        desired_ground_y,
        GroundGridDecisionHelper::DEFAULT_TOLERANCE,
    );
    let coverage_radius_changed = GroundGridDecisionHelper::needs_radius_update(
        grid.coverage_radius,
        desired_radius,
        GroundGridDecisionHelper::DEFAULT_TOLERANCE,
    );
    let visibility_changed = grid.visible != toggles.renderer.grid;
    let color_changed = grid.color != desired_color;

    if ground_y_changed {
        grid.ground_y = desired_ground_y;
    }
    if coverage_radius_changed {
        grid.coverage_radius = desired_radius;
    }
    if visibility_changed {
        grid.visible = toggles.renderer.grid;
    }
    if color_changed {
        grid.color = desired_color;
    }

    if let Some(ref mut counters) = counters {
        counters.grid_sync_calls += 1;
        for changed in [
            ground_y_changed,
            coverage_radius_changed,
            visibility_changed,
            color_changed,
        ] {
            if changed {
                counters.grid_host_writes += 1;
                counters.grid_value_changes += 1;
            }
        }
        if ground_y_changed {
            counters.grid_ground_y_writes += 1;
        }
        if coverage_radius_changed {
            counters.grid_coverage_radius_writes += 1;
        }
        if visibility_changed {
            counters.grid_visible_writes += 1;
        }
        if grid.is_changed() {
            counters.grid_changed_observations += 1;
        }
        if let Some(ref gc) = glacial_counters {
            counters.grid_update_alpha_calls = gc.alpha_rebuild_calls;
            counters.grid_lines_rebuilt = gc.lines_rebuilt;
            counters.grid_dots_rebuilt = gc.dots_rebuilt;
            counters.grid_structural_rebuilds = gc.alpha_rebuild_calls;
            counters.grid_vertices_generated = gc.vertices_generated;
            counters.grid_indices_generated = gc.indices_generated;
        }
    }
}

#[cfg(test)]
#[path = "visualization_environment_tests.rs"]
mod tests;
