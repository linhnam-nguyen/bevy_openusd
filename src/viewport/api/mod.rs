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
pub(in crate::viewport) use bridge::{
    ViewerSettingsState, ViewportBridgePlugin, ViewportBridgeSet,
};
pub(crate) use interface::RenderServerInterface;
pub(in crate::viewport) use query::LatestMailbox;
pub(crate) use queues::{
    ViewportCommandInbox, ViewportEventOutbox, ViewportTreeCommand, ViewportTreeCommandInbox,
};
pub(crate) use read_model::ViewportReadModelState;
pub(crate) use session_registry::SessionRegistry;

#[cfg(test)]
#[path = "../../../tests/unit/viewport/api/test_harness.rs"]
pub(in crate::viewport) mod test_harness;
