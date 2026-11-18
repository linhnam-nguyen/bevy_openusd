//! Render-server transport selection and adapters for the viewport process.

pub(crate) mod frame_capture;
pub(crate) mod webrtc;

pub(crate) use frame_capture::{
    FrameCapturePlugin, FrameReadbackCorrelation, FrameTransportResource,
};

/// The delivered viewport transport enabled for this launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewportTransport {
    /// Headless WebRTC streaming server.
    WebRtc,
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/transport/transport_tests.rs"]
mod tests;
