//! Bevy-side adapter for the UI-neutral viewport protocol.
//!
//! The shared `viewport_protocol` crate owns public data types. This module
//! owns in-process queues and will translate them into private ECS state.

mod bridge;
mod interface;
mod queues;
mod read_model;
mod scene_index;
mod scene_query;
mod session_registry;

pub(crate) use bridge::{ViewportBridgePlugin, ViewportBridgeSet};
pub(crate) use interface::RenderServerInterface;
pub(crate) use queues::{
    ViewportCommandInbox, ViewportEventOutbox, ViewportTreeCommand, ViewportTreeCommandInbox,
};
pub(crate) use read_model::ViewportReadModelState;
pub(crate) use scene_index::SceneAnchorIndex;
pub(crate) use scene_query::SceneQueryService;
pub(crate) use session_registry::SessionRegistry;
