//! GStreamer hardware-accelerated video encoding pipeline (H.265 / AV1 / H.264).
//!
//! Converts raw RGBA `VideoFrame`s into encoded RTP payload packets.
//! Automatically probes for available hardware encoders across Vulkan Video (VK_KHR_video_encode),
//! NVIDIA (NVENC), AMD (AMF/VAAPI), Apple (VideoToolbox), and software fallbacks.

mod caps;
mod pipeline;

pub use caps::{CodecCapabilities, VideoCodec};
pub use pipeline::EncodePipeline;

#[cfg(test)]
mod tests;
