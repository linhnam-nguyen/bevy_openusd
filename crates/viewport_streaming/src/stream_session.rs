//! Per-client WebRTC streaming-session ownership.
//!
//! A session owns the encoder pipeline, its webrtcbin, both DataChannels,
//! signaling sender, and teardown boundary. The frame pump is shared only as a
//! routing mechanism so a reconnect cannot consume frames through an old
//! session.

use anyhow::{Context, Result};
use gstreamer::prelude::*;
use log::{debug, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::sync::mpsc;
use viewport_protocol::{
    ActiveStreamConfiguration, CodecId, SessionId, ViewportMetrics, ViewportReadModel,
};

use crate::RenderServerInterface;
use crate::VideoFrame;
use crate::config::StreamingConfig;
use crate::data_channel::DataChannelSet;
use crate::encode::{EncodePipeline, VideoCodec};
use crate::signaling::SignalingMessage;

const EXPECTED_MEDIA_SECTIONS: u32 = 2;

/// Routes raw frames to the currently active encoder without exposing the
/// encoder or GStreamer objects to Bevy.
#[derive(Clone, Default)]
pub(crate) struct FrameRouter {
    target: Arc<Mutex<Option<ActiveFrameTarget>>>,
}

struct ActiveFrameTarget {
    connection_id: u64,
    encoder: Arc<EncodePipeline>,
    codec: CodecId,
    expected: Option<ExpectedInitialFrame>,
    current: Option<ActiveStreamConfiguration>,
    applied: Option<ActiveStreamConfiguration>,
}

#[derive(Clone)]
struct ExpectedInitialFrame {
    metrics: ViewportMetrics,
    caps_applied: bool,
}

impl FrameRouter {
    fn activate(&self, connection_id: u64, encoder: Arc<EncodePipeline>, codec: CodecId) {
        if let Ok(mut target) = self.target.lock() {
            *target = Some(ActiveFrameTarget {
                connection_id,
                encoder,
                codec,
                expected: None,
                current: None,
                applied: None,
            });
        }
    }

    fn deactivate(&self, connection_id: u64) {
        if let Ok(mut target) = self.target.lock() {
            if target
                .as_ref()
                .is_some_and(|active| active.connection_id == connection_id)
            {
                *target = None;
            }
        }
    }

    fn configure(&self, connection_id: u64, metrics: ViewportMetrics) {
        if let Ok(mut target) = self.target.lock()
            && let Some(active) = target.as_mut()
            && active.connection_id == connection_id
        {
            let newest_generation = active
                .expected
                .as_ref()
                .map(|expected| expected.metrics.generation)
                .into_iter()
                .chain(active.current.as_ref().map(|current| current.generation))
                .max()
                .unwrap_or(0);
            if metrics.generation <= newest_generation {
                warn!(
                    "[viewport-frame-pump] ignored stale stream configuration generation {} (newest {})",
                    metrics.generation, newest_generation
                );
                return;
            }
            active.expected = Some(ExpectedInitialFrame {
                metrics,
                caps_applied: false,
            });
            active.current = None;
            active.applied = None;
        }
    }

    fn take_applied(&self, connection_id: u64) -> Option<ActiveStreamConfiguration> {
        let mut target = self.target.lock().ok()?;
        let active = target.as_mut()?;
        if active.connection_id != connection_id {
            return None;
        }
        active.applied.take()
    }

    fn push(&self, frame: &VideoFrame) {
        let Some((encoder, codec, expected, current)) =
            self.target.lock().ok().and_then(|target| {
                target.as_ref().map(|active| {
                    (
                        Arc::clone(&active.encoder),
                        active.codec,
                        active.expected.clone(),
                        active.current.clone(),
                    )
                })
            })
        else {
            return;
        };

        if let Some(expected) = expected {
            if frame.width != expected.metrics.requested_width
                || frame.height != expected.metrics.requested_height
                || frame.generation != expected.metrics.generation
            {
                return;
            }

            let fps = expected.metrics.preferred_fps.unwrap_or(60);
            if !expected.caps_applied {
                if let Err(error) = encoder.set_video_caps(frame.width, frame.height, fps) {
                    warn!("[viewport-frame-pump] stream caps update failed: {error:?}");
                    return;
                }
                if let Err(error) = encoder.request_sync_frame_after_caps_change() {
                    warn!(
                        "[viewport-frame-pump] sync-frame/configuration refresh failed: {error:?}"
                    );
                    return;
                }
            }

            if let Err(error) = encoder.push_rgba_frame(&frame.rgba) {
                debug!("[viewport-frame-pump] frame push failed: {error:?}");
                return;
            }

            let configuration = ActiveStreamConfiguration {
                width: frame.width,
                height: frame.height,
                fps,
                codec,
                generation: frame.generation,
            };
            if let Ok(mut target) = self.target.lock()
                && let Some(active) = target.as_mut()
                && active
                    .expected
                    .as_ref()
                    .is_some_and(|current| current.metrics == expected.metrics)
            {
                active.expected = None;
                active.current = Some(configuration.clone());
                active.applied = Some(configuration);
            }
            return;
        }

        let Some(current) = current else {
            return;
        };
        if frame.width != current.width
            || frame.height != current.height
            || frame.generation != current.generation
        {
            return;
        }
        if let Err(error) = encoder.push_rgba_frame(&frame.rgba) {
            debug!("[viewport-frame-pump] frame push failed: {error:?}");
            return;
        }
    }
}

/// Owns the blocking Bevy-frame receiver and forwards frames to the active
/// session. Frames arriving while disconnected are intentionally dropped.
pub(crate) struct FramePump {
    router: FrameRouter,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl FramePump {
    pub(crate) fn new(receiver: Receiver<VideoFrame>) -> Self {
        let router = FrameRouter::default();
        let worker_router = router.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);

        let worker = thread::Builder::new()
            .name("viewport-frame-pump".to_owned())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(frame) => worker_router.push(&frame),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .expect("viewport frame pump thread should start");

        Self {
            router,
            stop,
            worker: Some(worker),
        }
    }

    pub(crate) fn router(&self) -> FrameRouter {
        self.router.clone()
    }
}

impl Drop for FramePump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// One isolated server-side WebRTC peer and media pipeline.
pub struct StreamingSession {
    connection_id: u64,
    session_id: SessionId,
    encoder: Arc<EncodePipeline>,
    channels: DataChannelSet,
    application: crate::data_channel::ApplicationSession,
    frame_router: FrameRouter,
}

impl StreamingSession {
    pub(crate) fn new(
        config: &StreamingConfig,
        connection_id: u64,
        reply_tx: mpsc::Sender<SignalingMessage>,
        frame_router: FrameRouter,
        runtime_handle: tokio::runtime::Handle,
        interface: RenderServerInterface,
        initial_viewport: Option<ViewportMetrics>,
    ) -> Result<Self> {
        let mut session_config = config.clone();
        if let Some(initial_viewport) = initial_viewport {
            let normalized = viewport_protocol::ServerCapabilities::for_codec(config.codec)
                .stream_limits
                .normalize(&initial_viewport);
            session_config.width = normalized.requested_width;
            session_config.height = normalized.requested_height;
            session_config.fps = normalized.preferred_fps.unwrap_or(config.fps);
            info!(
                "[viewport-session] using initial Join viewport {}x{} @ {} fps",
                session_config.width, session_config.height, session_config.fps
            );
        }

        let codec = VideoCodec::try_from(session_config.codec)?;
        let encoder = Arc::new(EncodePipeline::new(&session_config, codec)?);
        encoder.prepare_video_offer(session_config.width, session_config.height)?;
        let webrtc = encoder.webrtc();
        install_ice_forwarding(&webrtc, reply_tx, runtime_handle);

        let session_id = SessionId::new(format!("session-{connection_id}"));
        let application = crate::data_channel::ApplicationSession::new_with_capabilities(
            session_id.clone(),
            ViewportReadModel::unloaded(session_config.stage_display_name.clone()),
            interface.clone(),
            viewport_protocol::ServerCapabilities::for_codec(session_config.codec),
        );

        // DataChannel callbacks are installed before both local channels are
        // created, and both channels therefore appear in the generated offer.
        let channels = DataChannelSet::create(&webrtc, application.clone())?;
        frame_router.activate(connection_id, Arc::clone(&encoder), session_config.codec);

        info!(
            "[viewport-session] created session {} for signaling connection {}",
            session_id.0, connection_id
        );

        Ok(Self {
            connection_id,
            session_id,
            encoder,
            channels,
            application,
            frame_router,
        })
    }

    pub(crate) fn flush_authoritative_events(&self) {
        if let Some(metrics) = self.application.take_stream_configuration() {
            self.frame_router.configure(self.connection_id, metrics);
        }
        if let Some(configuration) = self.frame_router.take_applied(self.connection_id) {
            self.application.queue_configuration_applied(configuration);
        }
        self.application
            .flush_authoritative_events(self.channels.control());
    }

    pub(crate) async fn create_offer(&self) -> Result<String> {
        let (promise, promise_future) = gstreamer::Promise::new_future();
        self.encoder
            .webrtc()
            .emit_by_name::<()>("create-offer", &[&None::<gstreamer::Structure>, &promise]);

        let reply = promise_future
            .await
            .map_err(|error| anyhow::anyhow!("create-offer promise failed: {error:?}"))?
            .context("create-offer returned no reply")?;
        let offer = reply
            .get::<gstreamer_webrtc::WebRTCSessionDescription>("offer")
            .context("create-offer reply contained no SDP offer")?;

        self.encoder.webrtc().emit_by_name::<()>(
            "set-local-description",
            &[&offer, &None::<gstreamer::Promise>],
        );

        let offer_sdp = offer
            .sdp()
            .as_text()
            .context("failed to serialize generated SDP offer")?;
        let media_kinds = (0..offer.sdp().medias_len())
            .filter_map(|index| offer.sdp().media(index).and_then(|media| media.media()))
            .collect::<Vec<_>>();
        info!(
            "[viewport-session] created SDP offer with {} media sections: {:?}",
            offer.sdp().medias_len(),
            media_kinds
        );
        for index in 0..offer.sdp().medias_len() {
            let Some(media) = offer.sdp().media(index) else {
                continue;
            };
            if media.media() == Some("video") {
                let rtpmap = media.attribute_val("rtpmap").unwrap_or("<missing>");
                let fmtp = media.attribute_val("fmtp").unwrap_or("<missing>");
                info!(
                    "[viewport-session] video SDP payloads={:?}, rtpmap={rtpmap}, fmtp={fmtp}",
                    media.formats().collect::<Vec<_>>()
                );
            }
        }

        Ok(offer_sdp)
    }

    pub(crate) async fn apply_answer(&self, sdp: String) -> Result<()> {
        let sdp_message = gstreamer_sdp::SDPMessage::parse_buffer(sdp.as_bytes())
            .context("failed to parse remote SDP answer")?;
        let sdp_message = normalize_remote_answer(sdp_message, EXPECTED_MEDIA_SECTIONS)?;
        let answer = gstreamer_webrtc::WebRTCSessionDescription::new(
            gstreamer_webrtc::WebRTCSDPType::Answer,
            sdp_message,
        );
        let (promise, promise_future) = gstreamer::Promise::new_future();

        self.encoder
            .webrtc()
            .emit_by_name::<()>("set-remote-description", &[&answer, &promise]);
        promise_future
            .await
            .map_err(|error| anyhow::anyhow!("set-remote-description promise failed: {error:?}"))?;

        info!(
            "[viewport-session] applied SDP answer for session {}",
            self.session_id.0
        );
        Ok(())
    }

    pub(crate) fn apply_ice(
        &self,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u32>,
    ) {
        let Some(mline_index) = sdp_mline_index else {
            warn!(
                "[viewport-session] ignoring ICE candidate without m-line index for session {} (mid: {:?})",
                self.session_id.0, sdp_mid
            );
            return;
        };

        self.encoder
            .webrtc()
            .emit_by_name::<()>("add-ice-candidate", &[&mline_index, &candidate]);
        debug!(
            "[viewport-session] applied remote ICE candidate for session {} (mid: {:?}, index: {})",
            self.session_id.0, sdp_mid, mline_index
        );
    }
}

impl Drop for StreamingSession {
    fn drop(&mut self) {
        self.frame_router.deactivate(self.connection_id);
        self.channels.close();
        self.encoder.shutdown();
        info!(
            "[viewport-session] closed session {} for signaling connection {}",
            self.session_id.0, self.connection_id
        );
    }
}

/// Validates the browser answer before handing it to native `webrtcbin`.
///
/// Some browser implementations place the DTLS fingerprint at session scope,
/// while the native validator looks it up on each media section. Copying that
/// standards-permitted session attribute to each media section keeps the
/// boundary explicit and avoids sending an incomplete description into the
/// native plugin.
fn normalize_remote_answer(
    mut sdp: gstreamer_sdp::SDPMessage,
    expected_media_sections: u32,
) -> Result<gstreamer_sdp::SDPMessage> {
    let actual_media_sections = sdp.medias_len();
    if actual_media_sections != expected_media_sections {
        anyhow::bail!(
            "remote SDP answer has {actual_media_sections} media sections; expected {expected_media_sections}"
        );
    }

    let session_fingerprint = sdp.attribute_val("fingerprint").map(str::to_owned);

    for index in 0..actual_media_sections {
        let (media_kind, has_mid, has_ice_credentials, has_setup, has_fingerprint, port, has_sctp) = {
            let media = sdp
                .media(index)
                .with_context(|| format!("remote SDP answer is missing media section {index}"))?;
            (
                media.media().unwrap_or("unknown").to_owned(),
                media.attribute_val("mid").is_some(),
                media.attribute_val("ice-ufrag").is_some()
                    && media.attribute_val("ice-pwd").is_some(),
                media.attribute_val("setup").is_some(),
                media.attribute_val("fingerprint").is_some(),
                media.port(),
                media.attribute_val("sctp-port").is_some()
                    || media.attribute_val("sctpmap").is_some(),
            )
        };

        if !has_mid {
            anyhow::bail!("remote SDP media section {index} ({media_kind}) has no mid");
        }
        if !has_ice_credentials {
            anyhow::bail!(
                "remote SDP media section {index} ({media_kind}) has incomplete ICE credentials"
            );
        }
        if !has_setup {
            anyhow::bail!("remote SDP media section {index} ({media_kind}) has no DTLS setup role");
        }

        if !has_fingerprint {
            let Some(fingerprint) = session_fingerprint.as_deref() else {
                anyhow::bail!(
                    "remote SDP media section {index} ({media_kind}) has no DTLS fingerprint"
                );
            };
            sdp.media_mut(index)
                .expect("media section was checked above")
                .add_attribute("fingerprint", Some(fingerprint));
        }

        if media_kind == "video" && port == 0 {
            anyhow::bail!("remote SDP video section {index} was rejected");
        }
        if media_kind == "application" && !has_sctp {
            anyhow::bail!(
                "remote SDP application section {index} has no SCTP transport description"
            );
        }
    }

    Ok(sdp)
}

fn install_ice_forwarding(
    webrtc: &gstreamer::Element,
    reply_tx: mpsc::Sender<SignalingMessage>,
    runtime_handle: tokio::runtime::Handle,
) {
    webrtc.connect("on-ice-candidate", false, move |values| {
        let Ok(mline_index) = values
            .get(1)
            .ok_or(())
            .and_then(|value| value.get::<u32>().map_err(|_| ()))
        else {
            warn!("[viewport-session] invalid local ICE m-line index");
            return None;
        };
        let Ok(candidate) = values
            .get(2)
            .ok_or(())
            .and_then(|value| value.get::<String>().map_err(|_| ()))
        else {
            warn!("[viewport-session] invalid local ICE candidate");
            return None;
        };

        let reply_tx = reply_tx.clone();
        let _ = runtime_handle.spawn(async move {
            if reply_tx
                .send(SignalingMessage::Ice {
                    candidate,
                    sdp_mid: None,
                    sdp_mline_index: Some(mline_index),
                })
                .await
                .is_err()
            {
                warn!("[viewport-session] signaling peer closed before local ICE forwarding");
            }
        });
        None
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER_WITH_MEDIA_FINGERPRINTS: &str = "v=0\r\n\
o=- 1 1 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
a=group:BUNDLE 0 1\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
c=IN IP4 0.0.0.0\r\n\
a=mid:0\r\n\
a=ice-ufrag:ufrag\r\n\
a=ice-pwd:password\r\n\
a=fingerprint:sha-256 00\r\n\
a=setup:active\r\n\
a=recvonly\r\n\
m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n\
c=IN IP4 0.0.0.0\r\n\
a=mid:1\r\n\
a=ice-ufrag:ufrag\r\n\
a=ice-pwd:password\r\n\
a=fingerprint:sha-256 00\r\n\
a=setup:active\r\n\
a=sctp-port:5000\r\n";

    const ANSWER_WITH_SESSION_FINGERPRINT: &str = "v=0\r\n\
o=- 1 1 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
a=fingerprint:sha-256 00\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
c=IN IP4 0.0.0.0\r\n\
a=mid:0\r\n\
a=ice-ufrag:ufrag\r\n\
a=ice-pwd:password\r\n\
a=setup:active\r\n\
a=recvonly\r\n\
m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n\
c=IN IP4 0.0.0.0\r\n\
a=mid:1\r\n\
a=ice-ufrag:ufrag\r\n\
a=ice-pwd:password\r\n\
a=setup:active\r\n\
a=sctp-port:5000\r\n";

    #[test]
    fn remote_answer_with_media_fingerprints_is_accepted() {
        gstreamer::init().unwrap();
        let sdp =
            gstreamer_sdp::SDPMessage::parse_buffer(ANSWER_WITH_MEDIA_FINGERPRINTS.as_bytes())
                .unwrap();

        let normalized = normalize_remote_answer(sdp, EXPECTED_MEDIA_SECTIONS).unwrap();
        assert_eq!(normalized.medias_len(), EXPECTED_MEDIA_SECTIONS);
    }

    #[test]
    fn session_fingerprint_is_copied_to_each_media_section() {
        gstreamer::init().unwrap();
        let sdp =
            gstreamer_sdp::SDPMessage::parse_buffer(ANSWER_WITH_SESSION_FINGERPRINT.as_bytes())
                .unwrap();

        let normalized = normalize_remote_answer(sdp, EXPECTED_MEDIA_SECTIONS).unwrap();
        for index in 0..EXPECTED_MEDIA_SECTIONS {
            assert_eq!(
                normalized
                    .media(index)
                    .unwrap()
                    .attribute_val("fingerprint"),
                Some("sha-256 00")
            );
        }
    }

    #[test]
    fn remote_answer_with_wrong_media_count_is_rejected_before_webrtcbin() {
        gstreamer::init().unwrap();
        let sdp =
            gstreamer_sdp::SDPMessage::parse_buffer(ANSWER_WITH_MEDIA_FINGERPRINTS.as_bytes())
                .unwrap();

        let error = normalize_remote_answer(sdp, 1).unwrap_err();
        assert!(error.to_string().contains("expected 1"));
    }
}
