//! GStreamer hardware-accelerated video encoding pipeline (H.265 / AV1 / H.264).
//!
//! Converts raw RGBA frames from `FrameData` channel into encoded RTP payload packets.
//! Automatically probes for available hardware encoders across Vulkan Video (VK_KHR_video_encode),
//! NVIDIA (NVENC), AMD (AMF/VAAPI), Apple (VideoToolbox), and software fallbacks.

use anyhow::{Context, Result};
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;
use log::info;

use crate::config::StreamingConfig;

/// Supported video encoding formats negotiated over WebRTC SDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H265,
    AV1,
    H264,
}

/// Discovers available GStreamer encoder elements on the host OS / GPU.
#[derive(Debug, Clone)]
pub struct CodecCapabilities {
    pub h265_encoder: Option<String>,
    pub av1_encoder: Option<String>,
    pub h264_encoder: Option<String>,
}

impl CodecCapabilities {
    /// Probes GStreamer registry for hardware (Vulkan Video, NVIDIA, AMD, Apple) and software encoders.
    pub fn probe() -> Self {
        let _ = gstreamer::init();

        Self {
            h265_encoder: find_first_encoder(&[
                "vulkanh265enc", // Vulkan Video Encode (Cross-vendor: AMD/NVIDIA/Intel)
                "nvh265enc",     // NVIDIA NVENC (Linux/Windows)
                "amfh265enc",    // AMD AMF (Windows)
                "vah265enc",     // AMD/Intel VAAPI (Linux)
                "vtenc_h265",    // Apple VideoToolbox (macOS)
                "x265enc",       // Software fallback
            ]),
            av1_encoder: find_first_encoder(&[
                "vulkanav1enc", // Vulkan Video AV1 Encode (Cross-vendor)
                "nvav1enc",     // NVIDIA RTX 40+ NVENC
                "amfav1enc",    // AMD RDNA 3 / RX 7000+ AMF (Windows)
                "vaav1enc",     // AMD RDNA 3 / Intel Arc VAAPI (Linux)
                "svtav1enc",    // Software fallback
            ]),
            h264_encoder: find_first_encoder(&[
                "vulkanh264enc", // Vulkan Video H.264 Encode
                "nvh264enc",     // NVIDIA NVENC
                "amfh264enc",    // AMD AMF
                "vah264enc",     // AMD/Intel VAAPI
                "vtenc_h264",    // Apple VideoToolbox
                "x264enc",       // Software fallback
            ]),
        }
    }
}

fn find_first_encoder(candidates: &[&str]) -> Option<String> {
    for &name in candidates {
        if gstreamer::ElementFactory::find(name).is_some() {
            info!("[viewport-encode] Found encoder element: {name}");
            return Some(name.to_string());
        }
    }
    None
}

/// Managed GStreamer encoding pipeline instance.
pub struct EncodePipeline {
    pipeline: gstreamer::Pipeline,
    appsrc: AppSrc,
    selected_codec: VideoCodec,
}

impl EncodePipeline {
    /// Builds a low-latency GStreamer pipeline targeting the selected codec and resolution.
    pub fn new(config: &StreamingConfig, codec: VideoCodec) -> Result<Self> {
        gstreamer::init().context("Failed to initialize GStreamer")?;

        let caps = CodecCapabilities::probe();
        let encoder_name = match codec {
            VideoCodec::H265 => caps
                .h265_encoder
                .context("No H.265 encoder available in GStreamer registry")?,
            VideoCodec::AV1 => caps
                .av1_encoder
                .context("No AV1 encoder available in GStreamer registry")?,
            VideoCodec::H264 => caps
                .h264_encoder
                .context("No H.264 encoder available in GStreamer registry")?,
        };

        info!(
            "[viewport-encode] Creating pipeline with codec {:?} using encoder `{}`",
            codec, encoder_name
        );

        let pipeline = gstreamer::Pipeline::with_name("viewport-encode-pipeline");

        let appsrc = gstreamer::ElementFactory::make("appsrc")
            .name("video_src")
            .build()?
            .downcast::<AppSrc>()
            .map_err(|_| anyhow::anyhow!("Failed to downcast appsrc"))?;

        appsrc.set_caps(Some(
            &gstreamer_video::VideoCapsBuilder::new()
                .format(gstreamer_video::VideoFormat::Rgba)
                .width(config.width as i32)
                .height(config.height as i32)
                .framerate(gstreamer::Fraction::new(config.fps as i32, 1))
                .build(),
        ));
        appsrc.set_is_live(true);

        let videoconvert = gstreamer::ElementFactory::make("videoconvert").build()?;
        let encoder = gstreamer::ElementFactory::make(&encoder_name).build()?;

        // Configure low-latency properties depending on encoder type
        if encoder_name.contains("nv") || encoder_name.contains("amf") {
            let _ = encoder.set_property_from_str("preset", "low-latency-hq");
        } else if encoder_name.contains("vt") {
            let _ = encoder.set_property("realtime", true);
        }

        pipeline.add_many(&[&appsrc.upcast_ref(), &videoconvert, &encoder])?;
        gstreamer::Element::link_many(&[&appsrc.upcast_ref(), &videoconvert, &encoder])?;

        Ok(Self {
            pipeline,
            appsrc,
            selected_codec: codec,
        })
    }

    /// Pushes a raw RGBA frame from Bevy offscreen render target into the GStreamer pipeline.
    pub fn push_rgba_frame(&self, rgba_data: &[u8]) -> Result<()> {
        let mut buffer = gstreamer::Buffer::with_size(rgba_data.len())
            .context("Failed to allocate GStreamer buffer")?;

        {
            let buffer_ref = buffer.get_mut().unwrap();
            let mut map = buffer_ref
                .map_writable()
                .context("Failed to map buffer writable")?;
            map.copy_from_slice(rgba_data);
        }

        self.appsrc
            .push_buffer(buffer)
            .map_err(|_| anyhow::anyhow!("Failed to push buffer to appsrc"))?;

        Ok(())
    }

    pub fn selected_codec(&self) -> VideoCodec {
        self.selected_codec
    }
}
