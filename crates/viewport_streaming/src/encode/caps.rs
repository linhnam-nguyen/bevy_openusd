use anyhow::{Context, Result};
use log::info;
use viewport_protocol::CodecId;

const CODEC_PRIORITY: [CodecId; 3] = [CodecId::Av1, CodecId::H265, CodecId::H264];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EncoderTier {
    /// NVIDIA NVENC is a known discrete-GPU encoder family.
    KnownDiscreteGpu,
    /// Hardware encoder factory whose adapter type is not exposed by GStreamer.
    OtherHardware,
    Software,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderCandidate {
    pub codec: CodecId,
    pub factory_name: String,
    pub tier: EncoderTier,
}

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
    pub candidates: Vec<EncoderCandidate>,
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

        let candidates = [CodecId::Av1, CodecId::H265, CodecId::H264]
            .into_iter()
            .flat_map(encoder_candidates)
            .collect::<Vec<_>>();

        Self {
            h265_encoder: first_factory(&candidates, CodecId::H265),
            av1_encoder: first_factory(&candidates, CodecId::Av1),
            h264_encoder: first_factory(&candidates, CodecId::H264),
            candidates,
        }
    }

    /// Candidates are ranked GPU tier first, then AV1, H.265, and H.264.
    /// Software encoders are considered only after hardware candidates.
    pub fn auto_candidates(&self, client_codecs: &[CodecId]) -> Vec<EncoderCandidate> {
        let mut candidates = self
            .candidates
            .iter()
            .filter(|candidate| client_codecs.contains(&candidate.codec))
            .cloned()
            .collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| {
            (
                candidate.tier,
                CODEC_PRIORITY
                    .iter()
                    .position(|codec| *codec == candidate.codec)
                    .unwrap_or(usize::MAX),
            )
        });
        candidates
    }

    /// Returns every available host encoder for an explicitly requested codec.
    pub fn candidates_for_codec(&self, codec: CodecId) -> Vec<EncoderCandidate> {
        let mut candidates = self
            .candidates
            .iter()
            .filter(|candidate| candidate.codec == codec)
            .cloned()
            .collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| candidate.tier);
        candidates
    }
}

fn encoder_candidates(codec: CodecId) -> Vec<EncoderCandidate> {
    let factories: &[&str] = match codec {
        CodecId::Av1 => &[
            "nvav1enc",     // NVIDIA NVENC (discrete)
            "amfav1enc",    // AMD AMF (discrete GPU or APU)
            "qsvav1enc",    // Intel oneVPL/QSV (Arc or integrated)
            "vulkanav1enc", // Vulkan Video (adapter type varies)
            "vaav1enc",     // VA-API (adapter type varies)
            "svtav1enc",    // Software fallback
        ],
        CodecId::H265 => &[
            "nvh265enc",     // NVIDIA NVENC (discrete)
            "amfh265enc",    // AMD AMF (discrete GPU or APU)
            "qsvh265enc",    // Intel oneVPL/QSV (Arc or integrated)
            "vulkanh265enc", // Vulkan Video (adapter type varies)
            "vah265enc",     // VA-API (adapter type varies)
            "vtenc_h265",    // Apple VideoToolbox
            "x265enc",       // Software fallback
        ],
        CodecId::H264 => &[
            "nvh264enc",     // NVIDIA NVENC (discrete)
            "amfh264enc",    // AMD AMF (discrete GPU or APU)
            "qsvh264enc",    // Intel oneVPL/QSV (Arc or integrated)
            "vulkanh264enc", // Vulkan Video (adapter type varies)
            "vah264enc",     // VA-API (adapter type varies)
            "vtenc_h264",    // Apple VideoToolbox
            "x264enc",       // Software fallback
        ],
        CodecId::Vp8 | CodecId::Vp9 => return Vec::new(),
    };

    factories
        .iter()
        .filter(|name| gstreamer::ElementFactory::find(name).is_some())
        .map(|name| {
            let tier = classify_encoder(name);
            info!(
                "[viewport-encode] Found {:?} {:?} encoder element: {name}",
                codec, tier
            );
            EncoderCandidate {
                codec,
                factory_name: (*name).to_owned(),
                tier,
            }
        })
        .collect()
}

fn first_factory(candidates: &[EncoderCandidate], codec: CodecId) -> Option<String> {
    candidates
        .iter()
        .find(|candidate| candidate.codec == codec)
        .map(|candidate| candidate.factory_name.clone())
}

fn classify_encoder(factory_name: &str) -> EncoderTier {
    if factory_name.starts_with("nv") {
        EncoderTier::KnownDiscreteGpu
    } else if matches!(factory_name, "x264enc" | "x265enc" | "svtav1enc") {
        EncoderTier::Software
    } else {
        // AMF, QSV, Vulkan, VA-API, and VideoToolbox can target either
        // integrated or discrete adapters; the factory name alone does not
        // expose that distinction.
        EncoderTier::OtherHardware
    }
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
