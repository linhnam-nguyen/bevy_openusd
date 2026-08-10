//! GStreamer WebRTC video streaming and input bridge for USDHub viewport.
//!
//! Provides hardware-accelerated video encoding (H.265 / AV1 / H.264),
//! WebRTC SDP/ICE negotiation, WebSocket signaling server, and
//! DataChannel ↔ Bevy protocol message translation.

pub mod bridge;
pub mod config;
pub mod encode;
pub mod session;
pub mod signaling;

pub use config::{StreamingConfig, StreamingPreset};
pub use encode::{CodecCapabilities, EncodePipeline, VideoCodec};
pub use session::WebRtcSessionManager;
pub use signaling::{SignalingMessage, run_signaling_server};
