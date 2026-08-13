//! GStreamer hardware-accelerated video encoding pipeline (H.265 / AV1 / H.264).
//!
//! Converts raw RGBA frames from `FrameData` channel into encoded RTP payload packets.
//! Automatically probes for available hardware encoders across Vulkan Video (VK_KHR_video_encode),
//! NVIDIA (NVENC), AMD (AMF/VAAPI), Apple (VideoToolbox), and software fallbacks.

use anyhow::{Context, Result};
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;
use log::info;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
use viewport_protocol::CodecId;

use crate::config::StreamingConfig;

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
    _pipeline: gstreamer::Pipeline,
    webrtc: gstreamer::Element,
    webrtc_sink_pad: gstreamer::Pad,
    rtp_src_pad: gstreamer::Pad,
    appsrc: AppSrc,
    selected_codec: VideoCodec,
    active_caps: Mutex<(u32, u32, u32)>,
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

        let initial_caps = raw_video_caps(config.width, config.height, config.fps)?;
        appsrc.set_caps(Some(&initial_caps));
        appsrc.set_is_live(true);
        appsrc.set_format(gstreamer::Format::Time);
        appsrc.set_do_timestamp(true);

        let (parser_name, payloader_name) = match codec {
            VideoCodec::H264 => ("h264parse", "rtph264pay"),
            VideoCodec::H265 => ("h265parse", "rtph265pay"),
            VideoCodec::AV1 => ("av1parse", "rtpav1pay"),
        };

        let videoconvert = gstreamer::ElementFactory::make("videoconvert").build()?;
        let encoder = gstreamer::ElementFactory::make(&encoder_name).build()?;
        let parser = gstreamer::ElementFactory::make(parser_name).build()?;
        let codec_filter = gstreamer::ElementFactory::make("capsfilter").build()?;

        if codec == VideoCodec::H264 {
            let h264_caps = gstreamer::Caps::builder("video/x-h264")
                .field("profile", "constrained-baseline")
                .field("stream-format", "avc")
                .field("alignment", "au")
                .build();

            codec_filter.set_property("caps", &h264_caps);
        } else if codec == VideoCodec::AV1 {
            // rtpav1pay only accepts parsed low-overhead AV1 OBUs. Keep the
            // parser/payloader boundary deterministic instead of relying on
            // downstream caps negotiation to choose an alignment that the
            // browser's WebRTC AV1 receiver may not accept.
            let av1_caps = gstreamer::Caps::builder("video/x-av1")
                .field("parsed", true)
                .field("stream-format", "obu-stream")
                .field("alignment", "tu")
                .build();

            codec_filter.set_property("caps", &av1_caps);
        }
        let payloader = gstreamer::ElementFactory::make(payloader_name).build()?;
        if matches!(codec, VideoCodec::H264 | VideoCodec::H265) {
            // H.264/H.265 payloaders periodically repeat codec configuration
            // NAL units. rtpav1pay has no config-interval property.
            payloader.set_property("config-interval", 1i32);
        }
        if codec == VideoCodec::H265 {
            // RFC 7798's zero-latency aggregation keeps VPS/SPS/PPS with the
            // first VCL NAL instead of waiting for a full access unit.
            payloader.set_property_from_str("aggregate-mode", "zero-latency");
            payloader.set_property("config-interval", -1i32);
        }
        // A warm-up buffer is pushed before the WebRTC offer exists. Without
        // an asynchronous boundary here, webrtcbin can backpressure the
        // payloader while it is still waiting for SDP/ICE, which also blocks
        // appsrc from delivering every subsequent raw frame. Keep a bounded
        // RTP queue so encoder progress is independent of negotiation.
        let rtp_queue = gstreamer::ElementFactory::make("queue")
            .property("max-size-buffers", 512u32)
            .property("max-size-bytes", 16u32 * 1024 * 1024)
            .property("max-size-time", 1_000_000_000u64)
            .build()?;
        // Put the fixed RTP caps immediately before webrtcbin. This lets the
        // WebRTC sink advertise its video m-line during offer creation while
        // the queue still decouples the encoder from pre-ICE backpressure.
        let rtp_caps_filter = gstreamer::ElementFactory::make("capsfilter").build()?;
        rtp_caps_filter.set_property("caps", &rtp_video_caps(codec));
        let mut webrtc_builder = gstreamer::ElementFactory::make("webrtcbin").name("webrtcbin");
        // Browsers answer with a single BUNDLE transport for the video and
        // DataChannel m-lines. Keep webrtcbin on the same transport model so
        // its remote-description fingerprint validation sees one consistent
        // ICE/DTLS transport across both media sections.
        webrtc_builder = webrtc_builder.property_from_str("bundle-policy", "max-bundle");
        if !config.stun_server.is_empty() {
            webrtc_builder = webrtc_builder.property("stun-server", &config.stun_server);
        }
        let webrtc = webrtc_builder.build()?;

        // Configure low-latency properties depending on encoder type
        if encoder_name.contains("nv") || encoder_name.contains("amf") {
            let _ = encoder.set_property_from_str("preset", "low-latency-hq");
        } else if encoder_name.contains("x264") {
            let _ = encoder.set_property_from_str("tune", "zerolatency");
            // The default x264 speed preset is optimized for compression
            // efficiency rather than interactive throughput. The viewport
            // favors a fresh frame over compression density, so use the
            // fastest low-latency preset for the software fallback.
            let _ = encoder.set_property_from_str("speed-preset", "ultrafast");
        } else if encoder_name == "vtenc_h265" {
            // VideoToolbox provides the viable hardware alternative to AV1 on
            // this Mac. Disable B-frame reordering and select its realtime
            // CBR mode so H.265 has the same interactive intent as H.264.
            let _ = encoder.set_property("realtime", true);
            let _ = encoder.set_property("allow-frame-reordering", false);
            let _ = encoder.set_property("bitrate", config.h265_bitrate_kbps);
            let keyint = config.fps.saturating_mul(2).max(1);
            let _ = encoder.set_property("max-keyframe-interval", keyint as i32);
            let _ = encoder.set_property_from_str("rate-control", "cbr");
        } else if encoder_name == "svtav1enc" {
            // The installed GStreamer/SVT-AV1 combination can initialize the
            // low-delay `pred-struct=1` mode but does not drain normal encoded
            // frames from it; only the initial sequence unit reaches the RTP
            // payloader. Keep the stable random-access structure for now and
            // use the plugin's supported target-bitrate property. Preset 13 is
            // SVT-AV1's fastest mode, which is the AV1 equivalent of the
            // software H.264 `ultrafast` choice for this interactive viewport.
            let _ = encoder.set_property_from_str("preset", "13");
            let keyint = config.fps.saturating_mul(2).max(1);
            let _ = encoder.set_property("intra-period-length", keyint as i32);
            let _ = encoder.set_property("target-bitrate", config.av1_bitrate_kbps);
        }

        pipeline.add_many(&[
            appsrc.upcast_ref(),
            &videoconvert,
            &encoder,
            &parser,
            &codec_filter,
            &payloader,
            &rtp_queue,
            &rtp_caps_filter,
            &webrtc,
        ])?;

        gstreamer::Element::link_many(&[
            appsrc.upcast_ref(),
            &videoconvert,
            &encoder,
            &parser,
            &codec_filter,
            &payloader,
            &rtp_queue,
            &rtp_caps_filter,
        ])?;

        // Keep the first three buffers visible at every media boundary. The
        // AV1 RTP probe alone cannot distinguish an encoder stall from a
        // parser/payloader stall after the raw frame has been accepted.
        log_first_buffers(
            &appsrc
                .static_pad("src")
                .context("Failed to get appsrc src pad")?,
            "appsrc-src",
        );
        log_first_buffers(
            &encoder
                .static_pad("src")
                .context("Failed to get encoder src pad")?,
            "encoder-src",
        );
        log_first_buffers(
            &parser
                .static_pad("src")
                .context("Failed to get parser src pad")?,
            "parser-src",
        );
        log_first_buffers(
            &codec_filter
                .static_pad("src")
                .context("Failed to get codec-filter src pad")?,
            "codec-filter-src",
        );
        log_first_buffers(
            &payloader
                .static_pad("sink")
                .context("Failed to get payloader sink pad")?,
            "payloader-sink",
        );
        // Set pipeline state to Ready so webrtcbin initializes its pad templates
        pipeline.set_state(gstreamer::State::Ready)?;

        // Request sink_%u pad using webrtcbin's pad template
        let pad_templ = webrtc
            .pad_template("sink_%u")
            .context("Failed to find sink_%u pad template on webrtcbin")?;
        let sink_pad = webrtc
            .request_pad(&pad_templ, None, None)
            .context("Failed to request sink pad from webrtcbin")?;

        let rtp_src_pad = rtp_caps_filter
            .static_pad("src")
            .context("Failed to get src pad from RTP caps filter")?;

        rtp_src_pad
            .link(&sink_pad)
            .context("Failed to link RTP caps filter src pad to webrtcbin sink pad")?;

        let rtp_buffer_count = Arc::new(AtomicU64::new(0));
        let rtp_buffer_count_for_probe = Arc::clone(&rtp_buffer_count);
        rtp_src_pad.add_probe(gstreamer::PadProbeType::BUFFER, move |_pad, probe_info| {
            if let Some(gstreamer::PadProbeData::Buffer(buffer)) = &probe_info.data {
                let index = rtp_buffer_count_for_probe.fetch_add(1, Ordering::Relaxed);
                if index < 3 {
                    info!(
                        "[viewport-encode] {:?} RTP buffer #{}: {} bytes, pts={:?}",
                        codec,
                        index + 1,
                        buffer.size(),
                        buffer.pts()
                    );
                }
            }
            gstreamer::PadProbeReturn::Ok
        });

        if let Err(err) = pipeline.set_state(gstreamer::State::Playing) {
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
            anyhow::bail!(
                "Failed to set GStreamer pipeline state to Playing: {err:?}. Detail: {err_detail}"
            );
        }

        // The capsfilter declares the format, but an offer can be requested
        // before the first queued buffer has propagated its sticky CAPS event.
        // Push that event across the already-linked media pad once the pad is
        // active so webrtcbin can create the video m-line without relying on
        // a query result that has not become negotiated caps yet.
        if !rtp_src_pad.push_event(gstreamer::event::StreamStart::new("usd-hub-viewport")) {
            anyhow::bail!("Failed to push RTP stream-start event to webrtcbin");
        }
        if !rtp_src_pad.push_event(gstreamer::event::Caps::new(&rtp_video_caps(codec))) {
            anyhow::bail!("Failed to push RTP caps event to webrtcbin");
        }

        Ok(Self {
            _pipeline: pipeline,
            webrtc,
            webrtc_sink_pad: sink_pad,
            rtp_src_pad,
            appsrc,
            selected_codec: codec,
            active_caps: Mutex::new((config.width, config.height, config.fps)),
        })
    }

    pub fn webrtc(&self) -> gstreamer::Element {
        self.webrtc.clone()
    }

    /// Negotiates the video branch before creating the first WebRTC offer.
    ///
    /// A newly-created per-client pipeline has no real Bevy frame yet. Until
    /// one buffer reaches the payloader, webrtcbin has only created the
    /// DataChannel m-line, so an offer made at that point cannot describe
    /// video. A single black frame is enough to establish the RTP caps; the
    /// normal frame router replaces it as soon as the next rendered frame
    /// arrives.
    pub fn prepare_video_offer(&self, width: u32, height: u32) -> Result<()> {
        let pixel_count = (width as usize)
            .checked_mul(height as usize)
            .context("video dimensions overflow while preparing the WebRTC offer")?;
        let byte_count = pixel_count
            .checked_mul(4)
            .context("RGBA frame size overflow while preparing the WebRTC offer")?;

        let warmup_frame = vec![0; byte_count];
        // SVT-AV1 random-access mode buffers a mini-GOP before its first
        // displayable frame. Prime one GOP of black frames so the WebRTC
        // receiver gets a complete keyframe/configuration sequence while the
        // real Bevy frames are entering the same live pipeline.
        let warmup_frames = match self.selected_codec {
            VideoCodec::AV1 => 16,
            VideoCodec::H264 | VideoCodec::H265 => 1,
        };
        for _ in 0..warmup_frames {
            self.push_rgba_frame(&warmup_frame)?;
        }

        let deadline = Instant::now()
            + match self.selected_codec {
                VideoCodec::AV1 => Duration::from_secs(5),
                VideoCodec::H264 | VideoCodec::H265 => Duration::from_secs(2),
            };
        loop {
            if let Some(caps) = self.webrtc_sink_pad.current_caps()
                && !caps.is_any()
                && !caps.is_empty()
            {
                info!("[viewport-encode] video RTP caps negotiated before SDP offer: {caps:?}");
                return Ok(());
            }
            if let Some(caps) = self.rtp_src_pad.current_caps()
                && !caps.is_any()
                && !caps.is_empty()
            {
                info!(
                    "[viewport-encode] video RTP caps available from RTP caps filter before SDP offer: {caps:?}"
                );
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for video RTP caps before creating the WebRTC offer"
                );
            }

            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Pushes a raw RGBA frame from Bevy offscreen render target into the GStreamer pipeline.
    pub fn push_rgba_frame(&self, rgba_data: &[u8]) -> Result<()> {
        let mut buffer = gstreamer::Buffer::with_size(rgba_data.len())
            .context("Failed to allocate GStreamer buffer")?;

        {
            let buffer_ref = buffer.get_mut().unwrap();
            {
                let mut map = buffer_ref
                    .map_writable()
                    .context("Failed to map buffer writable")?;
                map.copy_from_slice(rgba_data);
            }

            // appsrc's do-timestamp supplies the running PTS, but raw buffers
            // otherwise have no duration. Supplying the frame period lets
            // GstVideoEncoder and downstream RTP timing advance continuously
            // for this live source.
            let fps = self
                .active_caps
                .lock()
                .map_err(|_| anyhow::anyhow!("video caps state lock poisoned"))?
                .2;
            if fps > 0 {
                buffer_ref.set_duration(gstreamer::ClockTime::from_nseconds(
                    1_000_000_000u64 / fps as u64,
                ));
            }
        }

        self.appsrc
            .push_buffer(buffer)
            .map_err(|_| anyhow::anyhow!("Failed to push buffer to appsrc"))?;

        Ok(())
    }

    /// Updates the raw RGBA caps before the first active frame for a session.
    /// `appsrc` exposes this as a caps event on the live source, so the first
    /// matching buffer is encoded with the same dimensions as the Bevy target.
    pub fn set_video_caps(&self, width: u32, height: u32, fps: u32) -> Result<()> {
        let mut active_caps = self
            .active_caps
            .lock()
            .map_err(|_| anyhow::anyhow!("video caps state lock poisoned"))?;
        if *active_caps == (width, height, fps) {
            return Ok(());
        }

        let caps = raw_video_caps(width, height, fps)?;
        self.appsrc.set_caps(Some(&caps));
        *active_caps = (width, height, fps);
        Ok(())
    }

    pub fn selected_codec(&self) -> VideoCodec {
        self.selected_codec
    }

    /// Stops the pipeline before its owning streaming session is dropped.
    pub fn shutdown(&self) {
        let _ = self._pipeline.set_state(gstreamer::State::Null);
    }
}

fn raw_video_caps(width: u32, height: u32, fps: u32) -> Result<gstreamer::Caps> {
    if width < 2 || height < 2 || width % 2 != 0 || height % 2 != 0 || fps == 0 {
        anyhow::bail!("invalid raw video caps {width}x{height}@{fps}");
    }
    Ok(gstreamer_video::VideoCapsBuilder::new()
        .format(gstreamer_video::VideoFormat::Rgba)
        .width(width as i32)
        .height(height as i32)
        .framerate(gstreamer::Fraction::new(fps as i32, 1))
        .build())
}

fn rtp_video_caps(codec: VideoCodec) -> gstreamer::Caps {
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

fn log_first_buffers(pad: &gstreamer::Pad, label: &'static str) {
    let buffer_count = Arc::new(AtomicU64::new(0));
    let buffer_count_for_probe = Arc::clone(&buffer_count);
    pad.add_probe(gstreamer::PadProbeType::BUFFER, move |_pad, probe_info| {
        if let Some(gstreamer::PadProbeData::Buffer(buffer)) = &probe_info.data {
            let index = buffer_count_for_probe.fetch_add(1, Ordering::Relaxed);
            if index < 3 {
                info!(
                    "[viewport-encode] {label} buffer #{}: {} bytes, pts={:?}, duration={:?}",
                    index + 1,
                    buffer.size(),
                    buffer.pts(),
                    buffer.duration()
                );
            }
        }
        gstreamer::PadProbeReturn::Ok
    });
}

impl Drop for EncodePipeline {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_creation() {
        let config = StreamingConfig::default();
        let pipeline = EncodePipeline::new(&config, VideoCodec::H264);
        let pipeline = pipeline.expect("Failed to create H.264 encode pipeline");
        assert_eq!(
            pipeline
                .webrtc()
                .property::<gstreamer_webrtc::WebRTCBundlePolicy>("bundle-policy"),
            gstreamer_webrtc::WebRTCBundlePolicy::MaxBundle
        );
    }

    #[test]
    fn av1_pipeline_creation_skips_h26x_config_interval() {
        let config = StreamingConfig::default();
        if CodecCapabilities::probe().av1_encoder.is_none() {
            return;
        }

        let pipeline = EncodePipeline::new(&config, VideoCodec::AV1)
            .expect("AV1 encoder is present but the AV1 pipeline could not be created");
        assert_eq!(pipeline.selected_codec(), VideoCodec::AV1);
    }

    #[test]
    fn av1_pipeline_can_negotiate_rtp_caps() {
        let config = StreamingConfig::from_preset(crate::config::StreamingPreset::Adaptive);
        if CodecCapabilities::probe().av1_encoder.is_none() {
            return;
        }

        let pipeline = EncodePipeline::new(&config, VideoCodec::AV1)
            .expect("AV1 encoder is present but the AV1 pipeline could not be created");
        pipeline
            .prepare_video_offer(config.width, config.height)
            .expect("AV1 pipeline did not negotiate RTP caps");
    }
}
