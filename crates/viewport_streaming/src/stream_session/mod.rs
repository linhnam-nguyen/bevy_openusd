//! Per-client WebRTC streaming-session ownership.
//!
//! A session owns the encoder pipeline, its webrtcbin, both DataChannels,
//! signaling sender, and teardown boundary. The frame pump is shared only as a
//! routing mechanism so a reconnect cannot consume frames through an old
//! session.

mod pump;
mod router;
mod session;

pub(crate) use pump::FramePump;
pub use session::StreamingSession;

#[cfg(test)]
mod tests;
