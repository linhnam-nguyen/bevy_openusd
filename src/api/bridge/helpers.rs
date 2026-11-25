//! Bridge helpers shared by API command and query adapters.
//! Read-model construction and protocol event publication live in separate submodules.

mod events;
mod read_model;

pub(super) use events::{
    emit_classification_field_catalogue, emit_editor_completed, emit_editor_export,
    emit_presentation_changed, emit_runtime_mutation_accepted, emit_selection_delta, emit_snapshot,
    emit_viewer_settings_changed, reject,
};
pub(super) use read_model::{build_read_model, presentation_read_model, timeline_read_model};

use crate::viewport::rendering::scene::SceneAnchorIndex;
use crate::viewport::rendering::scene::visualization::DisplayToggles;
use viewport_protocol::{OverlayKind, RenderMode, SceneAnchor};

pub(super) fn resolve_anchor(
    anchor: &SceneAnchor,
    scene_index: &SceneAnchorIndex,
) -> Result<bevy::ecs::entity::Entity, String> {
    scene_index.resolve(anchor).ok_or_else(|| {
        format!(
            "target {} is not present in the active scene",
            anchor.prim_path
        )
    })
}

pub(super) fn set_overlay(toggles: &mut DisplayToggles, overlay: OverlayKind, enabled: bool) {
    match overlay {
        OverlayKind::GroundGrid => toggles.renderer.grid = enabled,
        OverlayKind::WorldAxes => toggles.show_world_axes = enabled,
        OverlayKind::PrimMarkers => toggles.show_prim_markers = enabled,
        OverlayKind::Skeleton => toggles.show_skeleton = enabled,
        OverlayKind::Physics => toggles.show_physics = enabled,
        OverlayKind::Colliders => toggles.show_colliders = enabled,
        OverlayKind::Wireframe => {
            toggles.renderer.render_mode = if enabled {
                RenderMode::Wireframe
            } else {
                RenderMode::Shaded
            };
        }
    }
}
