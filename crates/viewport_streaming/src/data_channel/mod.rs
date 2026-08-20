//! WebRTC DataChannel construction and lifecycle diagnostics.
//!
//! The server creates both application channels before generating its SDP
//! offer. The reliable control channel also owns the application handshake;
//! semantic viewport commands remain queued for the Phase 3 bridge.

mod channel_set;
mod chunks;
mod constants;
mod dispatch;
mod events;
mod session;

pub use channel_set::{ChannelOptions, DataChannelSet};
pub use constants::{
    CONTROL_CHANNEL_LABEL, CONTROL_CHANNEL_PROTOCOL, INPUT_CHANNEL_LABEL, INPUT_CHANNEL_PROTOCOL,
};
pub(crate) use session::ApplicationSession;

#[cfg(test)]
mod tests;
