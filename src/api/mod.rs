//! Bevy-side adapter for the UI-neutral viewport protocol.
//!
//! The shared `viewport_protocol` crate owns public data types. This module
//! owns in-process queues and will translate them into private ECS state.

mod bridge;
mod interface;
mod query;
mod queues;
mod read_model;
mod session_registry;

pub(crate) use bridge::editor_value_to_usd;
pub(crate) use bridge::{
    ViewerSettingsState, ViewportBridgePlugin, ViewportBridgeSet, reset_document_edit_state,
};
pub(crate) use interface::RenderServerInterface;
pub(crate) use query::LatestMailbox;
pub(crate) use queues::{
    ViewportCommandInbox, ViewportEventOutbox, ViewportTreeCommand, ViewportTreeCommandInbox,
};
pub(crate) use read_model::{
    BimCatalogueLoadState, BimPropertiesLoadState, ViewportReadModelState,
};
pub(crate) use session_registry::SessionRegistry;

#[cfg(test)]
#[path = "../../tests/unit/viewport/api/test_harness.rs"]
pub(crate) mod test_harness;

pub(crate) use bridge::product_stage_lifecycle_hooks;
