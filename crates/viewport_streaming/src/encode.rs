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

fn init_gstreamer_env() {
    #[cfg(target_os = "macos")]
    {
        use std::env;
        let mut paths = Vec::new();
        if let Ok(existing) = env::var("GST_PLUGIN_PATH") {
            paths.push(existing);
        }
        paths.push("/opt/homebrew/opt/libnice-gstreamer/libexec/gstreamer-1.0".to_string());
        paths.push("/opt/homebrew/lib/gstreamer-1.0".to_string());
        if let Ok(joined) = env::join_paths(paths.iter()) {
            unsafe {
                env::set_var("GST_PLUGIN_PATH", joined);
            }
        }

        if env::var("DYLD_FALLBACK_LIBRARY_PATH").is_err() {
            unsafe {
                env::set_var("DYLD_FALLBACK_LIBRARY_PATH", "/opt/homebrew/lib");
            }
        }
    }
}

impl CodecCapabilities {
    /// Probes GStreamer registry for hardware (Vulkan Video, NVIDIA, AMD, Apple) and software encoders.
    pub fn probe() -> Self {
        init_gstreamer_env();
        let _ = gstreamer::init();

        Self {
            h265_encoder: find_first_encoder(&[
                "vulkanh265enc", // Vulkan Video Encode (Cross-vendor: AMD/NVIDIA/Intel)
                "nvh265enc",     // NVIDIA NVENC (Linux/Windows)
                "amfh265enc",    // AMD AMF (Windows)
                "vah265enc",     // AMD/Intel VAAPI (Linux)
                // "vtenc_h265", // Apple VideoToolbox (macOS)
                "vtenc_h264", // Hardware H.264 fallback on macOS
                "x265enc",    // Software fallback
            ]),
            av1_encoder: find_first_encoder(&[
                "vulkanav1enc", // Vulkan Video AV1 Encode (Cross-vendor)
                "nvav1enc",     // NVIDIA RTX 40+ NVENC
                "amfav1enc",    // AMD RDNA 3 / RX 7000+ AMF (Windows)
                "vaav1enc",     // AMD RDNA 3 / Intel Arc VAAPI (Linux)
                "svtav1enc",    // Software fallback
                "vtenc_h264",
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
                .or(caps.h264_encoder.clone())
                .context("No H.265 / H.264 encoder available in GStreamer registry")?,
            VideoCodec::AV1 => caps
                .av1_encoder
                .or(caps.h264_encoder.clone())
                .context("No AV1 / H.264 encoder available in GStreamer registry")?,
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

        let (parser_name, payloader_name) = if encoder_name.contains("264") {
            ("h264parse", "rtph264pay")
        } else if encoder_name.contains("av1") {
            ("av1parse", "rtpav1pay")
        } else {
            ("h265parse", "rtph265pay")
        };

        let videoconvert = gstreamer::ElementFactory::make("videoconvert").build()?;
        let encoder = gstreamer::ElementFactory::make(&encoder_name).build()?;
        let parser = gstreamer::ElementFactory::make(parser_name).build()?;
        let payloader = gstreamer::ElementFactory::make(payloader_name)
            .property("config-interval", 1)
            .build()?;
        let mut webrtc_builder = gstreamer::ElementFactory::make("webrtcbin").name("webrtcbin");
        if !config.stun_server.is_empty() {
            webrtc_builder = webrtc_builder.property("stun-server", &config.stun_server);
        }
        let webrtc = webrtc_builder.build()?;

        // Configure low-latency properties depending on encoder type
        if encoder_name.contains("nv") || encoder_name.contains("amf") {
            let _ = encoder.set_property_from_str("preset", "low-latency-hq");
        } else if encoder_name.contains("x264") {
            let _ = encoder.set_property_from_str("tune", "zerolatency");
        }

        pipeline.add_many(&[
            appsrc.upcast_ref(),
            &videoconvert,
            &encoder,
            &parser,
            &payloader,
            &webrtc,
        ])?;

        gstreamer::Element::link_many(&[
            appsrc.upcast_ref(),
            &videoconvert,
            &encoder,
            &parser,
            &payloader,
        ])?;

        // Set pipeline state to Ready so webrtcbin initializes its pad templates
        pipeline.set_state(gstreamer::State::Ready)?;

        // Request sink_%u pad using webrtcbin's pad template
        let pad_templ = webrtc
            .pad_template("sink_%u")
            .context("Failed to find sink_%u pad template on webrtcbin")?;
        let sink_pad = webrtc
            .request_pad(&pad_templ, None, None)
            .context("Failed to request sink pad from webrtcbin")?;

        let src_pad = payloader
            .static_pad("src")
            .context("Failed to get src pad from payloader")?;

        src_pad
            .link(&sink_pad)
            .context("Failed to link payloader src pad to webrtcbin sink pad")?;

        if let Err(err) = pipeline.set_state(gstreamer::State::Ready) {
            let mut err_detail = String::new();
            if let Some(bus) = pipeline.bus() {
                if let Some(msg) = bus.timed_pop_filtered(
                    gstreamer::ClockTime::from_mseconds(100),
                    &[gstreamer::MessageType::Error],
                ) {
                    if let gstreamer::MessageView::Error(err_msg) = msg.view() {
                        err_detail = format!(
                            "{}: {} (debug: {:?})",
                            err_msg.src().map(|s| s.path_string()).unwrap_or_default(),
                            err_msg.error(),
                            err_msg.debug()
                        );
                    }
                }
            }
            anyhow::bail!("Failed to set GStreamer pipeline state to Ready: {err:?}. Detail: {err_detail}");
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_creation() {
        let config = StreamingConfig::default();
        let pipeline = EncodePipeline::new(&config, VideoCodec::H264);
        assert!(pipeline.is_ok(), "Failed to create H.264 encode pipeline: {:?}", pipeline.err());
    }
}

