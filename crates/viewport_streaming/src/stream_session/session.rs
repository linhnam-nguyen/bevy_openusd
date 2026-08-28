use anyhow::{Context, Result};
use gstreamer::prelude::*;
use log::{debug, info, warn};
use std::sync::Arc;
use tokio::sync::mpsc;
use viewport_protocol::{SessionId, ViewportMetrics, ViewportReadModel};

use crate::RenderServerInterface;
use crate::config::StreamingConfig;
use crate::data_channel::DataChannelSet;
use crate::encode::{EncodePipeline, VideoCodec};
use crate::session::SessionAdmission;
use crate::signaling::SignalingMessage;

use super::router::FrameRouter;

pub(super) const EXPECTED_MEDIA_SECTIONS: u32 = 2;

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
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        config: &StreamingConfig,
        connection_id: u64,
        reply_tx: mpsc::Sender<SignalingMessage>,
        frame_router: FrameRouter,
        runtime_handle: tokio::runtime::Handle,
        interface: RenderServerInterface,
        initial_viewport: Option<ViewportMetrics>,
        admission: SessionAdmission,
    ) -> Result<Self> {
        config
            .authorization
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid streaming authorization policy: {error}"))?;
        let mut session_config = config.clone();
        if let Some(shared_metrics) = frame_router.current_metrics() {
            session_config.width = shared_metrics.requested_width;
            session_config.height = shared_metrics.requested_height;
            session_config.fps = shared_metrics.preferred_fps.unwrap_or(config.fps);
            info!(
                "[viewport-session] joining shared stream {}x{} @ {} fps",
                session_config.width, session_config.height, session_config.fps
            );
        } else if let Some(initial_viewport) = initial_viewport {
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
            session_config.authorization.clone(),
            admission,
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
        self.application.refresh_authorization();
        self.application.refresh_semantic_sync_status();
        if let Some(metrics) = self.application.take_stream_configuration() {
            self.frame_router.configure(self.connection_id, metrics);
        }
        if let Some(configuration) = self.frame_router.take_applied(self.connection_id) {
            self.application.queue_configuration_applied(configuration);
        }
        self.application
            .flush_project_activation_results(self.channels.control());
        self.application
            .flush_authoritative_events(self.channels.control());
    }

    pub(crate) fn queue_authoritative_event(
        &self,
        event: viewport_protocol::ViewportEventEnvelope,
    ) {
        self.application.queue_authoritative_event(event);
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
        self.application.release_admission();
        self.frame_router.deactivate(self.connection_id);
        self.channels.close();
        self.encoder.shutdown();
        info!(
            "[viewport-session] closed session {} for signaling connection {}",
            self.session_id.0, self.connection_id
        );
    }
}

pub(super) fn normalize_remote_answer(
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
        std::mem::drop(runtime_handle.spawn(async move {
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
        }));
        None
    });
}
