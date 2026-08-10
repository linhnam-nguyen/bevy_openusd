//! Bevy-side adapter for the UI-neutral viewport protocol.
//!
//! The shared `viewport_protocol` crate owns public data types. This module
//! owns in-process queues and will translate them into private ECS state.

mod bridge;
mod queues;
mod scene_index;

pub(crate) use bridge::{ViewportBridgePlugin, ViewportBridgeSet};
pub(crate) use queues::{
    ViewportCommandInbox, ViewportEventOutbox, ViewportTreeCommand, ViewportTreeCommandInbox,
};
pub(crate) use scene_index::SceneAnchorIndex;
