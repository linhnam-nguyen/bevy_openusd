use anyhow::{Context, Result};
use log::info;
use viewport_protocol::CodecId;

/// Supported video encoding formats negotiated over WebRTC SDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H265,
    AV1,
    H264,
}

impl TryFrom<CodecId> for VideoCodec {
    type Error = anyhow::Error;

    fn try_from(codec: CodecId) -> Result<Self> {
        match codec {
            CodecId::H264 => Ok(Self::H264),
            CodecId::H265 => Ok(Self::H265),
            CodecId::Av1 => Ok(Self::AV1),
            CodecId::Vp8 | CodecId::Vp9 => {
                anyhow::bail!("viewport streaming does not provide a {:?} encoder", codec)
            }
        }
    }
}

impl From<VideoCodec> for CodecId {
    fn from(codec: VideoCodec) -> Self {
        match codec {
            VideoCodec::H264 => Self::H264,
            VideoCodec::H265 => Self::H265,
            VideoCodec::AV1 => Self::Av1,
        }
    }
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
                "x264enc",       // Software fallback
                "vtenc_h264",    // Apple VideoToolbox
            ]),
        }
    }
}

pub(super) fn find_first_encoder(candidates: &[&str]) -> Option<String> {
    for &name in candidates {
        if gstreamer::ElementFactory::find(name).is_some() {
            info!("[viewport-encode] Found encoder element: {name}");
            return Some(name.to_string());
        }
    }
    None
}

pub(super) fn raw_video_caps(width: u32, height: u32, fps: u32) -> Result<gstreamer::Caps> {
    if width < 2 || height < 2 || !width.is_multiple_of(2) || !height.is_multiple_of(2) || fps == 0
    {
        anyhow::bail!("invalid raw video caps {width}x{height}@{fps}");
    }
    Ok(gstreamer_video::VideoCapsBuilder::new()
        .format(gstreamer_video::VideoFormat::Rgba)
        .width(width as i32)
        .height(height as i32)
        .framerate(gstreamer::Fraction::new(fps as i32, 1))
        .build())
}

pub(super) fn rgba_byte_count(width: u32, height: u32) -> Result<usize> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .context("video dimensions overflow while calculating RGBA frame size")
}

pub(super) fn rtp_video_caps(codec: VideoCodec) -> gstreamer::Caps {
    let encoding_name = match codec {
        VideoCodec::H264 => "H264",
        VideoCodec::H265 => "H265",
        VideoCodec::AV1 => "AV1",
    };

    gstreamer::Caps::builder("application/x-rtp")
        .field("media", "video")
        .field("clock-rate", 90_000i32)
        .field("encoding-name", encoding_name)
        .field("payload", 96i32)
        .build()
}

pub(super) fn sync_frame_event() -> gstreamer::Event {
    gstreamer_video::UpstreamForceKeyUnitEvent::builder()
        .all_headers(true)
        .build()
}
