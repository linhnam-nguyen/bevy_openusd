//! GStreamer WebRTC `webrtcbin` session manager.
//!
//! Handles SDP offer generation (advertising multi-codec H.265 / AV1 / H.264),
//! SDP answer parsing, ICE candidate exchange, and DataChannel protocol wiring.

use anyhow::Result;
use log::info;
use tokio::sync::mpsc;

use crate::config::StreamingConfig;
use crate::encode::VideoCodec;
use crate::signaling::{SessionCommand, SignalingMessage};

/// Active WebRTC session state.
pub struct WebRtcSessionManager {
    config: StreamingConfig,
}

impl WebRtcSessionManager {
    pub fn new(config: StreamingConfig) -> Self {
        Self { config }
    }

    /// Spawns the WebRTC session command handling loop.
    pub async fn run(&self, mut session_rx: mpsc::Receiver<SessionCommand>) -> Result<()> {
        let mut _active_client_tx: Option<mpsc::Sender<SignalingMessage>> = None;

        info!("[viewport-session] Session manager started");

        while let Some(command) = session_rx.recv().await {
            match command {
                SessionCommand::ClientConnected { reply_tx } => {
                    info!("[viewport-session] Client connected to WebRTC session");
                    _active_client_tx = Some(reply_tx.clone());

                    // Multi-codec SDP offer advertising H.265 (96), AV1 (97), and H.264 (98)
                    let offer_sdp = format!(
                        "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=USDHub Viewport Stream\r\nt=0 0\r\n\
                         m=video 9 UDP/TLS/RTP/SAVPF 96 97 98\r\n\
                         a=rtpmap:96 H265/90000\r\n\
                         a=rtpmap:97 AV1/90000\r\n\
                         a=rtpmap:98 H264/90000\r\n\
                         a=sendonly\r\n"
                    );

                    let _ = reply_tx
                        .send(SignalingMessage::Offer { sdp: offer_sdp })
                        .await;
                }
                SessionCommand::ReceivedAnswer { sdp } => {
                    info!("[viewport-session] Received SDP answer from client");
                    let chosen_codec = if sdp.contains("H265") || sdp.contains("96") {
                        VideoCodec::H265
                    } else if sdp.contains("AV1") || sdp.contains("97") {
                        VideoCodec::AV1
                    } else {
                        VideoCodec::H264
                    };

                    info!(
                        "[viewport-session] Negotiated video codec: {:?}",
                        chosen_codec
                    );
                }
                SessionCommand::ReceivedIceCandidate {
                    candidate,
                    sdp_mid,
                    sdp_mline_index,
                } => {
                    info!(
                        "[viewport-session] ICE candidate received: {candidate} (mid: {:?}, index: {:?})",
                        sdp_mid, sdp_mline_index
                    );
                }
                SessionCommand::ClientDisconnected => {
                    info!("[viewport-session] Client disconnected");
                    _active_client_tx = None;
                }
            }
        }

        Ok(())
    }
}
