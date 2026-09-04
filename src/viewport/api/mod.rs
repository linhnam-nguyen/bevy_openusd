//! Bevy-side adapter for the UI-neutral viewport protocol.
//!
//! The shared `viewport_protocol` crate owns public data types. This module
//! owns in-process queues and will translate them into private ECS state.

mod bim_provenance;
mod bridge;
mod hierarchy;
mod hierarchy_visibility;
mod interface;
mod queues;
mod read_model;
mod scene_index;
mod scene_occurrence_index;
mod scene_query;
mod session_registry;

pub(crate) use bim_provenance::BimProvenanceService;
pub(crate) use bridge::editor_value_to_usd;
#[cfg(test)]
pub(in crate::viewport) use bridge::refresh_active_hierarchy_projection;
pub(in crate::viewport) use bridge::{
    ViewerSettingsState, ViewportBridgePlugin, ViewportBridgeSet,
};
pub(crate) use hierarchy::{
    ActiveHierarchyProvider, BimClassificationRecipeState, CurrentHierarchyProjection,
    HierarchyPageIndex, HierarchyVisibilityIndex, HierarchyVisibilityTarget,
};
pub(crate) use hierarchy_visibility::refresh_projection_visibility;
pub(crate) use interface::RenderServerInterface;
pub(crate) use queues::{
    ViewportCommandInbox, ViewportEventOutbox, ViewportTreeCommand, ViewportTreeCommandInbox,
};
pub(crate) use read_model::ViewportReadModelState;
pub(crate) use scene_index::SceneAnchorIndex;
pub(crate) use session_registry::SessionRegistry;
