//! WebRTC signaling-to-streaming session coordination.
//!
//! Signaling connections carry only SDP, ICE, and lifecycle messages. Once a
//! connection is accepted, this manager creates one isolated StreamingSession.

use anyhow::Result;
use log::{error, info, warn};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc::Receiver},
    time::Duration,
};
use tokio::sync::mpsc;
use viewport_protocol::{SessionId, SessionRole};

use crate::RenderServerInterface;
use crate::VideoFrame;
use crate::config::StreamingConfig;
use crate::signaling::{SessionCommand, SignalingMessage};
use crate::stream_session::{FramePump, StreamingSession};

/// Coordinates signaling lifecycle with per-client GStreamer sessions.
pub struct WebRtcSessionManager {
    config: StreamingConfig,
    frame_pump: FramePump,
    interface: RenderServerInterface,
}

/// Shared application-session admission: one controller and any number of
/// observers. The renderer and frame source remain shared elsewhere.
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionAdmission {
    state: Arc<Mutex<SessionAdmissionState>>,
}

#[derive(Debug, Default)]
struct SessionAdmissionState {
    roles: HashMap<SessionId, SessionRole>,
    controller: Option<SessionId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionAdmissionError {
    ControllerAlreadyAssigned,
    SessionAlreadyRegistered,
}

impl SessionAdmission {
    pub(crate) fn register(
        &self,
        session_id: SessionId,
        role: SessionRole,
    ) -> Result<(), SessionAdmissionError> {
        let mut state = self
            .state
            .lock()
            .expect("session admission state is not poisoned");
        if state.roles.contains_key(&session_id) {
            return Err(SessionAdmissionError::SessionAlreadyRegistered);
        }
        if role == SessionRole::Controller && state.controller.is_some() {
            return Err(SessionAdmissionError::ControllerAlreadyAssigned);
        }
        if role == SessionRole::Controller {
            state.controller = Some(session_id.clone());
        }
        state.roles.insert(session_id, role);
        Ok(())
    }

    pub(crate) fn unregister(&self, session_id: &SessionId) -> bool {
        let mut state = self
            .state
            .lock()
            .expect("session admission state is not poisoned");
        let removed = state.roles.remove(session_id).is_some();
        if state.controller.as_ref() == Some(session_id) {
            state.controller = None;
        }
        removed
    }

    #[cfg(test)]
    fn role(&self, session_id: &SessionId) -> Option<SessionRole> {
        self.state
            .lock()
            .expect("session admission state is not poisoned")
            .roles
            .get(session_id)
            .copied()
    }
}

impl WebRtcSessionManager {
    pub fn new(
        config: StreamingConfig,
        frame_receiver: Receiver<VideoFrame>,
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
        let admission = SessionAdmission::default();
        let mut sessions: HashMap<u64, StreamingSession> = HashMap::new();
        let mut event_tick = tokio::time::interval(Duration::from_millis(16));

        info!("[viewport-session] session manager started");

        loop {
            let next_command = tokio::select! {
                command = session_rx.recv() => command,
                _ = event_tick.tick() => {
                    if !sessions.is_empty() {
                        while let Some(event) = self.interface.pop_viewport_event() {
                            for session in sessions.values() {
                                session.queue_authoritative_event(event.clone());
                            }
                        }
                    }
                    for session in sessions.values() {
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
                    initial_viewport,
                } => {
                    if sessions.contains_key(&connection_id) {
                        send_error(
                            &reply_tx,
                            "connection_already_registered",
                            "signaling connection already has a streaming session",
                        )
                        .await;
                        warn!(
                            "[viewport-session] rejected duplicate connection {}",
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
                        initial_viewport,
                        admission.clone(),
                    ) {
                        Ok(session) => session,
                        Err(error) => {
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
                        warn!(
                            "[viewport-session] signaling peer closed before offer for connection {}",
                            connection_id
                        );
                        continue;
                    }

                    sessions.insert(connection_id, session);
                }
                SessionCommand::ReceivedAnswer { connection_id, sdp } => {
                    let Some(session) = sessions.get(&connection_id) else {
                        warn!(
                            "[viewport-session] ignored SDP answer from unknown connection {}",
                            connection_id
                        );
                        continue;
                    };

                    if let Err(error) = session.apply_answer(sdp).await {
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
                    let Some(session) = sessions.get(&connection_id) else {
                        warn!(
                            "[viewport-session] ignored ICE candidate from unknown connection {}",
                            connection_id
                        );
                        continue;
                    };

                    session.apply_ice(candidate, sdp_mid, sdp_mline_index);
                }
                SessionCommand::ClientDisconnected { connection_id } => {
                    if sessions.remove(&connection_id).is_some() {
                        info!(
                            "[viewport-session] disconnected streaming session {}",
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

        drop(sessions);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_controller_and_multiple_observers_are_admitted() {
        let admission = SessionAdmission::default();
        let controller = SessionId::new("controller");
        let observer_a = SessionId::new("observer-a");
        let observer_b = SessionId::new("observer-b");

        admission
            .register(controller.clone(), SessionRole::Controller)
            .unwrap();
        admission
            .register(observer_a.clone(), SessionRole::Observer)
            .unwrap();
        admission
            .register(observer_b.clone(), SessionRole::Observer)
            .unwrap();

        assert_eq!(admission.role(&controller), Some(SessionRole::Controller));
        assert_eq!(admission.role(&observer_a), Some(SessionRole::Observer));
        assert_eq!(admission.role(&observer_b), Some(SessionRole::Observer));
    }

    #[test]
    fn second_controller_is_rejected_without_replacing_the_first() {
        let admission = SessionAdmission::default();
        let first = SessionId::new("controller-1");
        admission
            .register(first.clone(), SessionRole::Controller)
            .unwrap();

        assert_eq!(
            admission.register(SessionId::new("controller-2"), SessionRole::Controller),
            Err(SessionAdmissionError::ControllerAlreadyAssigned)
        );
        assert_eq!(admission.role(&first), Some(SessionRole::Controller));
    }

    #[test]
    fn unregistering_controller_allows_a_new_controller() {
        let admission = SessionAdmission::default();
        let first = SessionId::new("controller-1");
        admission
            .register(first.clone(), SessionRole::Controller)
            .unwrap();
        assert!(admission.unregister(&first));
        admission
            .register(SessionId::new("controller-2"), SessionRole::Controller)
            .unwrap();
    }
}
