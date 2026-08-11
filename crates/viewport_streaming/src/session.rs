//! GStreamer WebRTC `webrtcbin` session manager.
//!
//! Handles SDP offer generation (advertising multi-codec H.265 / AV1 / H.264),
//! SDP answer parsing, ICE candidate exchange, and DataChannel protocol wiring.

use crate::signaling::{SessionCommand, SignalingMessage};
use anyhow::{Context, Result};
use gstreamer::prelude::*;
use log::{info, warn};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// Active WebRTC session state.
pub struct WebRtcSessionManager {
    webrtc: gstreamer::Element,
}

impl WebRtcSessionManager {
    pub fn new(webrtc: gstreamer::Element) -> Self {
        Self { webrtc }
    }

    /// Spawns the WebRTC session command handling loop.
    pub async fn run(&self, mut session_rx: mpsc::Receiver<SessionCommand>) -> Result<()> {
        let active_client_tx = Arc::new(Mutex::new(None::<mpsc::Sender<SignalingMessage>>));

        let ice_client_tx = Arc::clone(&active_client_tx);
        let runtime_handle = tokio::runtime::Handle::current();

        self.webrtc
            .connect("on-ice-candidate", false, move |values| {
                let Ok(mline_index) = values[1].get::<u32>() else {
                    warn!("[viewport-session] Invalid local ICE m-line index");
                    return None;
                };

                let Ok(candidate) = values[2].get::<String>() else {
                    warn!("[viewport-session] Invalid local ICE candidate");
                    return None;
                };

                let reply_tx = ice_client_tx.lock().ok().and_then(|active| active.clone());

                if let Some(reply_tx) = reply_tx {
                    let message = SignalingMessage::Ice {
                        candidate,
                        sdp_mid: None,
                        sdp_mline_index: Some(mline_index),
                    };

                    std::mem::drop(runtime_handle.spawn(async move {
                        if reply_tx.send(message).await.is_err() {
                            warn!("[viewport-session] Failed to forward local ICE candidate");
                        }
                    }));
                }

                None
            });
        info!("[viewport-session] Session manager started");

        while let Some(command) = session_rx.recv().await {
            match command {
                SessionCommand::ClientConnected { reply_tx } => {
                    if let Ok(mut active) = active_client_tx.lock() {
                        *active = Some(reply_tx.clone());
                    }
                    let (promise, promise_future) = gstreamer::Promise::new_future();

                    self.webrtc.emit_by_name::<()>(
                        "create-offer",
                        &[&None::<gstreamer::Structure>, &promise],
                    );

                    let reply = promise_future
                        .await
                        .map_err(|err| anyhow::anyhow!("create-offer promise failed: {err:?}"))?
                        .context("create-offer returned no reply")?;

                    let offer = reply
                        .get::<gstreamer_webrtc::WebRTCSessionDescription>("offer")
                        .context("create-offer reply contained no SDP offer")?;

                    self.webrtc.emit_by_name::<()>(
                        "set-local-description",
                        &[&offer, &None::<gstreamer::Promise>],
                    );

                    let offer_sdp = offer
                        .sdp()
                        .as_text()
                        .context("Failed to serialize generated SDP offer")?;

                    reply_tx
                        .send(SignalingMessage::Offer { sdp: offer_sdp })
                        .await
                        .context("Failed to send generated SDP offer")?;
                }
                SessionCommand::ReceivedAnswer { sdp } => {
                    info!("[viewport-session] Received SDP answer from client");

                    let sdp_message = gstreamer_sdp::SDPMessage::parse_buffer(sdp.as_bytes())
                        .context("Failed to parse remote SDP answer")?;

                    let answer = gstreamer_webrtc::WebRTCSessionDescription::new(
                        gstreamer_webrtc::WebRTCSDPType::Answer,
                        sdp_message,
                    );

                    let (promise, promise_future) = gstreamer::Promise::new_future();

                    self.webrtc
                        .emit_by_name::<()>("set-remote-description", &[&answer, &promise]);

                    promise_future.await.map_err(|err| {
                        anyhow::anyhow!("set-remote-description promise failed: {err:?}")
                    })?;

                    info!("[viewport-session] Remote SDP answer applied");
                }
                SessionCommand::ReceivedIceCandidate {
                    candidate,
                    sdp_mid,
                    sdp_mline_index,
                } => {
                    let Some(mline_index) = sdp_mline_index else {
                        warn!(
                            "[viewport-session] Ignoring ICE candidate without m-line index \
             (mid: {sdp_mid:?})"
                        );
                        continue;
                    };

                    self.webrtc
                        .emit_by_name::<()>("add-ice-candidate", &[&mline_index, &candidate]);

                    info!(
                        "[viewport-session] Applied remote ICE candidate \
         (mid: {sdp_mid:?}, index: {mline_index})"
                    );
                }
                SessionCommand::ClientDisconnected => {
                    info!("[viewport-session] Client disconnected");
                    if let Ok(mut active) = active_client_tx.lock() {
                        *active = None;
                    }
                }
            }
        }

        Ok(())
    }
}
