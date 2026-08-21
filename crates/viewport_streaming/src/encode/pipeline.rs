use anyhow::{Context, Result};
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;
use log::info;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::VideoFrame;
use crate::config::StreamingConfig;

use super::caps::{
    CodecCapabilities, VideoCodec, raw_video_caps, rgba_byte_count, rtp_video_caps,
    sync_frame_event,
};

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
            let av1_caps = gstreamer::Caps::builder("video/x-av1")
                .field("parsed", true)
                .field("stream-format", "obu-stream")
                .field("alignment", "tu")
                .build();

            codec_filter.set_property("caps", &av1_caps);
        }
        let payloader = gstreamer::ElementFactory::make(payloader_name).build()?;
        if matches!(codec, VideoCodec::H264 | VideoCodec::H265) {
            payloader.set_property("config-interval", 1i32);
        }
        if codec == VideoCodec::H265 {
            payloader.set_property_from_str("aggregate-mode", "zero-latency");
            payloader.set_property("config-interval", -1i32);
        }
        let rtp_queue = gstreamer::ElementFactory::make("queue")
            .property("max-size-buffers", 512u32)
            .property("max-size-bytes", 16u32 * 1024 * 1024)
            .property("max-size-time", 1_000_000_000u64)
            .build()?;
        let rtp_caps_filter = gstreamer::ElementFactory::make("capsfilter").build()?;
        rtp_caps_filter.set_property("caps", &rtp_video_caps(codec));
        let mut webrtc_builder = gstreamer::ElementFactory::make("webrtcbin").name("webrtcbin");
        webrtc_builder = webrtc_builder.property_from_str("bundle-policy", "max-bundle");
        if !config.stun_server.is_empty() {
            webrtc_builder = webrtc_builder.property("stun-server", &config.stun_server);
        }
        let webrtc = webrtc_builder.build()?;

        if encoder_name.contains("nv") || encoder_name.contains("amf") {
            let _ = encoder.set_property_from_str("preset", "low-latency-hq");
        } else if encoder_name.contains("x264") {
            let _ = encoder.set_property_from_str("tune", "zerolatency");
            let _ = encoder.set_property_from_str("speed-preset", "ultrafast");
        } else if encoder_name == "vtenc_h265" {
            let _ = encoder.set_property("realtime", true);
            let _ = encoder.set_property("allow-frame-reordering", false);
            let _ = encoder.set_property("bitrate", config.h265_bitrate_kbps);
            let keyint = config.fps.saturating_mul(2).max(1);
            let _ = encoder.set_property("max-keyframe-interval", keyint as i32);
            let _ = encoder.set_property_from_str("rate-control", "cbr");
        } else if encoder_name == "svtav1enc" {
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

        pipeline.set_state(gstreamer::State::Ready)?;

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
    pub fn prepare_video_offer(&self, width: u32, height: u32) -> Result<()> {
        let pixel_count = (width as usize)
            .checked_mul(height as usize)
            .context("video dimensions overflow while preparing the WebRTC offer")?;
        let byte_count = pixel_count
            .checked_mul(4)
            .context("RGBA frame size overflow while preparing the WebRTC offer")?;

        let warmup_frame = vec![0; byte_count];
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
        self.push_rgba_frame_with_trace(rgba_data, None)
    }

    /// Pushes a traced frame while preserving its monotonic capture timestamp
    /// for transport metrics while leaving live media timestamps to appsrc.
    pub fn push_frame(&self, frame: &VideoFrame) -> Result<()> {
        // Correlation identity and stage timestamps stay on FrameTrace. The
        // live appsrc owns the media running clock; absolute process-relative
        // PTS values would make WebRTC wait for a timestamp it cannot align to
        // its pipeline base time.
        self.push_rgba_frame(&frame.rgba)
    }

    fn push_rgba_frame_with_trace(
        &self,
        rgba_data: &[u8],
        timestamp_ns: Option<u64>,
    ) -> Result<()> {
        let (width, height, _) = *self
            .active_caps
            .lock()
            .map_err(|_| anyhow::anyhow!("video caps state lock poisoned"))?;
        let expected_bytes = rgba_byte_count(width, height)?;
        if rgba_data.len() != expected_bytes {
            anyhow::bail!(
                "refusing {}-byte RGBA frame under active {}x{} caps; expected {} bytes",
                rgba_data.len(),
                width,
                height,
                expected_bytes
            );
        }
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
            if let Some(timestamp_ns) = timestamp_ns {
                buffer_ref.set_pts(gstreamer::ClockTime::from_nseconds(timestamp_ns));
            }
        }

        self.appsrc
            .push_buffer(buffer)
            .map_err(|_| anyhow::anyhow!("Failed to push buffer to appsrc"))?;

        Ok(())
    }

    /// Updates the raw RGBA caps before the first active frame for a session.
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

    /// Requests a new independently decodable frame after a live raw-caps change.
    pub fn request_sync_frame_after_caps_change(&self) -> Result<()> {
        let event = sync_frame_event();
        if !self.rtp_src_pad.send_event(event) {
            anyhow::bail!("GStreamer rejected the upstream sync-frame request");
        }
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

impl Drop for EncodePipeline {
    fn drop(&mut self) {
        self.shutdown();
    }
}
