//! GStreamer WebRTC video streaming and input bridge for USDHub viewport.
//!
//! Provides hardware-accelerated video encoding (H.265 / AV1 / H.264),
//! WebRTC SDP/ICE negotiation, WebSocket signaling server, and
//! DataChannel ↔ Bevy protocol message translation.

pub mod application;
pub mod bridge;
pub mod channel_backpressure;
pub mod config;
pub mod data_channel;
pub mod encode;
pub mod session;
pub mod signaling;
pub mod stream_session;

pub use application::{RenderServerInterface, RenderServerPortError};
pub use config::{StreamingConfig, StreamingPreset};
pub use data_channel::{
    CONTROL_CHANNEL_LABEL, CONTROL_CHANNEL_PROTOCOL, INPUT_CHANNEL_LABEL, INPUT_CHANNEL_PROTOCOL,
};
pub use encode::{CodecCapabilities, EncodePipeline, VideoCodec};
pub use session::WebRtcSessionManager;
pub use signaling::{SignalingMessage, run_signaling_server};
pub use stream_session::StreamingSession;

/// Raw RGBA frame plus the Bevy target metadata required to gate the initial
/// stream configuration.
#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub generation: u64,
}
