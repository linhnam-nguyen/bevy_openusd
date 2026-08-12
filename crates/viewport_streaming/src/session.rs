//! WebRTC signaling-to-streaming session coordination.
//!
//! Signaling connections carry only SDP, ICE, and lifecycle messages. Once a
//! connection is accepted, this manager creates one isolated StreamingSession
//! and enforces the first implementation's one-controller policy.

use anyhow::Result;
use log::{error, info, warn};
use std::sync::mpsc::Receiver;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::RenderServerInterface;
use crate::config::StreamingConfig;
use crate::signaling::{SessionCommand, SignalingMessage};
use crate::stream_session::{FramePump, StreamingSession};

/// Coordinates signaling lifecycle with per-client GStreamer sessions.
pub struct WebRtcSessionManager {
    config: StreamingConfig,
    frame_pump: FramePump,
    interface: RenderServerInterface,
}

impl WebRtcSessionManager {
    pub fn new(
        config: StreamingConfig,
        frame_receiver: Receiver<Vec<u8>>,
        interface: RenderServerInterface,
    ) -> Self {
        Self {
            config,
            frame_pump: FramePump::new(frame_receiver),
            interface,
        }
    }

    pub async fn run(self, mut session_rx: mpsc::Receiver<SessionCommand>) -> Result<()> {
        let runtime_handle = tokio::runtime::Handle::current();
        let frame_router = self.frame_pump.router();
        let mut gate = ConnectionGate::default();
        let mut active: Option<StreamingSession> = None;
        let mut event_tick = tokio::time::interval(Duration::from_millis(16));

        info!("[viewport-session] session manager started");

        loop {
            let next_command = tokio::select! {
                command = session_rx.recv() => command,
                _ = event_tick.tick() => {
                    if let Some(session) = active.as_ref() {
                        session.flush_authoritative_events();
                    }
                    continue;
                }
            };

            let Some(command) = next_command else {
                break;
            };

            match command {
                SessionCommand::ClientConnected {
                    connection_id,
                    reply_tx,
                } => {
                    if !gate.try_claim(connection_id) {
                        send_error(
                            &reply_tx,
                            "resource_busy",
                            "viewport already has an active controller",
                        )
                        .await;
                        warn!(
                            "[viewport-session] rejected second controller connection {}",
                            connection_id
                        );
                        continue;
                    }

                    let session = match StreamingSession::new(
                        &self.config,
                        connection_id,
                        reply_tx.clone(),
                        frame_router.clone(),
                        runtime_handle.clone(),
                        self.interface.clone(),
                    ) {
                        Ok(session) => session,
                        Err(error) => {
                            gate.release_if(connection_id);
                            send_error(&reply_tx, "session_creation_failed", &error.to_string())
                                .await;
                            error!(
                                "[viewport-session] failed to create connection {}: {error:?}",
                                connection_id
                            );
                            continue;
                        }
                    };

                    let offer = match session.create_offer().await {
                        Ok(offer) => offer,
                        Err(error) => {
                            gate.release_if(connection_id);
                            send_error(&reply_tx, "offer_creation_failed", &error.to_string())
                                .await;
                            error!(
                                "[viewport-session] failed to create offer for connection {}: {error:?}",
                                connection_id
                            );
                            continue;
                        }
                    };

                    if reply_tx
                        .send(SignalingMessage::Offer { sdp: offer })
                        .await
                        .is_err()
                    {
                        gate.release_if(connection_id);
                        warn!(
                            "[viewport-session] signaling peer closed before offer for connection {}",
                            connection_id
                        );
                        continue;
                    }

                    active = Some(session);
                }
                SessionCommand::ReceivedAnswer { connection_id, sdp } => {
                    if !gate.is_active(connection_id) {
                        warn!(
                            "[viewport-session] ignored SDP answer from stale connection {}",
                            connection_id
                        );
                        continue;
                    }

                    if let Some(session) = active.as_ref()
                        && let Err(error) = session.apply_answer(sdp).await
                    {
                        error!(
                            "[viewport-session] failed to apply SDP answer for connection {}: {error:?}",
                            connection_id
                        );
                    }
                }
                SessionCommand::ReceivedIceCandidate {
                    connection_id,
                    candidate,
                    sdp_mid,
                    sdp_mline_index,
                } => {
                    if !gate.is_active(connection_id) {
                        warn!(
                            "[viewport-session] ignored ICE candidate from stale connection {}",
                            connection_id
                        );
                        continue;
                    }

                    if let Some(session) = active.as_ref() {
                        session.apply_ice(candidate, sdp_mid, sdp_mline_index);
                    }
                }
                SessionCommand::ClientDisconnected { connection_id } => {
                    if gate.release_if(connection_id) {
                        active.take();
                        info!(
                            "[viewport-session] disconnected active controller {}",
                            connection_id
                        );
                    } else {
                        warn!(
                            "[viewport-session] ignored stale disconnect from connection {}",
                            connection_id
                        );
                    }
                }
            }
        }

        drop(active);
        info!("[viewport-session] session manager stopped");
        Ok(())
    }
}

async fn send_error(reply_tx: &mpsc::Sender<SignalingMessage>, code: &str, message: &str) {
    let _ = reply_tx
        .send(SignalingMessage::Error {
            code: code.to_owned(),
            message: message.to_owned(),
        })
        .await;
}

#[derive(Debug, Default)]
struct ConnectionGate {
    active: Option<u64>,
}

impl ConnectionGate {
    fn try_claim(&mut self, connection_id: u64) -> bool {
        if self.active.is_some() {
            return false;
        }
        self.active = Some(connection_id);
        true
    }

    fn is_active(&self, connection_id: u64) -> bool {
        self.active == Some(connection_id)
    }

    fn release_if(&mut self, connection_id: u64) -> bool {
        if self.is_active(connection_id) {
            self.active = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_controller_is_rejected_without_replacing_the_first() {
        let mut gate = ConnectionGate::default();
        assert!(gate.try_claim(1));
        assert!(!gate.try_claim(2));
        assert!(gate.is_active(1));
        assert!(!gate.is_active(2));
    }

    #[test]
    fn stale_disconnect_cannot_clear_the_current_controller() {
        let mut gate = ConnectionGate::default();
        assert!(gate.try_claim(7));
        assert!(!gate.release_if(8));
        assert!(gate.is_active(7));
        assert!(gate.release_if(7));
        assert!(!gate.is_active(7));
    }
}
