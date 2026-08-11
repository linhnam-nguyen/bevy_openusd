//! Bevy-side adapter for the UI-neutral viewport protocol.
//!
//! The shared `viewport_protocol` crate owns public data types. This module
//! owns in-process queues and will translate them into private ECS state.

mod bridge;
mod interface;
mod queues;
mod scene_index;
mod session_registry;

pub(crate) use bridge::{ViewportBridgePlugin, ViewportBridgeSet};
#[allow(unused_imports)]
pub(crate) use interface::{
    RenderServerCommandPort, RenderServerEventPort, RenderServerInterface, RenderServerPortError,
};
pub(crate) use queues::{
    ViewportCommandInbox, ViewportEventOutbox, ViewportTreeCommand, ViewportTreeCommandInbox,
};
pub(crate) use scene_index::SceneAnchorIndex;
pub(crate) use session_registry::SessionRegistry;
