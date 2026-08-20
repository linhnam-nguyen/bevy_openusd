use gstreamer_webrtc::WebRTCDataChannel;
use log::error;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use viewport_protocol::{
    ActiveStreamConfiguration, AuthorizationPolicy, HandshakeRejectionReason,
    SemanticSyncOperation, SemanticSyncStatus, ServerCapabilities, ServerEvent,
    ServerEventEnvelope, SessionEvent, SessionId, SessionRole, StreamEvent, ViewportEventEnvelope,
    ViewportReadModel,
};

use super::constants::MAX_RECENT_REQUEST_IDS;
use super::dispatch::flush_pending_server_events;
use super::events::queue_server_event_for_request;
use crate::application::{RenderServerInterface, SemanticSyncRequest, SemanticSyncRequestKind};
use crate::session::{SessionAdmission, SessionAdmissionError};

pub(super) struct ApplicationSessionState {
    pub(super) session_id: SessionId,
    pub(super) client_name: String,
    pub(super) admission: SessionAdmission,
    pub(super) role: Option<SessionRole>,
    pub(super) server_capabilities: ServerCapabilities,
    pub(super) authorization: AuthorizationPolicy,
    pub(super) semantic_sync_status: Option<SemanticSyncStatus>,
    pub(super) initial_snapshot: ViewportReadModel,
    pub(super) interface: RenderServerInterface,
    pub(super) pending_stream_configuration: Option<viewport_protocol::ViewportMetrics>,
    pub(super) latest_stream_generation: u64,
    pub(super) client_sequence: u64,
    pub(super) server_sequence: u64,
    pub(super) handshaken: bool,
    pub(super) recent_request_ids: VecDeque<String>,
    pub(super) pending_server_events: VecDeque<ServerEventEnvelope>,
}

/// Application state shared by the two callbacks attached to one control
/// DataChannel. It deliberately knows only the transport-neutral protocol.
#[derive(Clone)]
pub(crate) struct ApplicationSession {
    pub(super) state: Arc<Mutex<ApplicationSessionState>>,
}

impl ApplicationSession {
    #[cfg(test)]
    pub(crate) fn new(
        session_id: SessionId,
        initial_snapshot: ViewportReadModel,
        interface: RenderServerInterface,
    ) -> Self {
        Self::new_with_capabilities(
            session_id,
            initial_snapshot,
            interface,
            ServerCapabilities::default(),
            AuthorizationPolicy::default(),
            SessionAdmission::default(),
        )
    }

    pub(crate) fn new_with_capabilities(
        session_id: SessionId,
        initial_snapshot: ViewportReadModel,
        interface: RenderServerInterface,
        server_capabilities: ServerCapabilities,
        authorization: AuthorizationPolicy,
        admission: SessionAdmission,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(ApplicationSessionState {
                session_id,
                client_name: String::new(),
                admission,
                role: None,
                server_capabilities,
                authorization,
                semantic_sync_status: None,
                initial_snapshot,
                interface,
                pending_stream_configuration: None,
                latest_stream_generation: 0,
                client_sequence: 0,
                server_sequence: 1,
                handshaken: false,
                recent_request_ids: VecDeque::new(),
                pending_server_events: VecDeque::new(),
            })),
        }
    }

    pub(crate) fn release_admission(&self) {
        if let Ok(state) = self.state.lock() {
            state.admission.unregister(&state.session_id);
            if state.handshaken {
                let request = SemanticSyncRequest {
                    request_id: format!("disconnect-{}", state.session_id.0),
                    session_id: state.session_id.clone(),
                    client_name: state.client_name.clone(),
                    authorization: state.authorization.clone(),
                    kind: SemanticSyncRequestKind::Client(SemanticSyncOperation::Close),
                };
                if let Err(error) = state
                    .interface
                    .submit_semantic_sync_control_request(request)
                {
                    error!(
                        "[viewport-data-channel] failed to queue control Close on disconnect for session {}: {error:?}",
                        state.session_id.0
                    );
                }
            }
        }
    }

    pub(crate) fn clear_remote_input(&self) {
        if let Ok(state) = self.state.lock() {
            if state.role == Some(SessionRole::Controller) {
                state.interface.clear_remote_input();
            }
        }
    }

    pub(crate) fn queue_authoritative_event(&self, event: ViewportEventEnvelope) {
        let Ok(mut state) = self.state.lock() else {
            error!("[viewport-data-channel] application session state is poisoned");
            return;
        };
        if state.handshaken {
            queue_server_event_for_request(
                &mut state,
                event.request_id,
                ServerEvent::Viewport(event.event),
            );
        }
    }

    pub(crate) fn take_stream_configuration(&self) -> Option<viewport_protocol::ViewportMetrics> {
        self.state.lock().ok()?.pending_stream_configuration.take()
    }

    pub(crate) fn queue_configuration_applied(&self, configuration: ActiveStreamConfiguration) {
        let Ok(mut state) = self.state.lock() else {
            error!("[viewport-data-channel] application session state is poisoned");
            return;
        };
        if state.handshaken {
            queue_server_event_for_request(
                &mut state,
                None,
                ServerEvent::Stream(StreamEvent::ConfigurationApplied { configuration }),
            );
        }
    }

    pub(crate) fn refresh_authorization(&self) {
        let Ok(mut state) = self.state.lock() else {
            error!("[viewport-data-channel] application session state is poisoned");
            return;
        };
        if !state.handshaken {
            return;
        }

        let authorization = state.interface.authorization_policy();
        if authorization == state.authorization {
            return;
        }

        let semantic_sync_request = SemanticSyncRequest {
            request_id: format!("authorization-change-{}", state.session_id.0),
            session_id: state.session_id.clone(),
            client_name: state.client_name.clone(),
            authorization: authorization.clone(),
            kind: SemanticSyncRequestKind::AuthorizationChanged,
        };

        if let Err(error) = state
            .interface
            .submit_semantic_sync_control_request(semantic_sync_request)
        {
            error!(
                "[viewport-data-channel] failed to queue semantic-sync control authorization change for session {}: {error:?}",
                state.session_id.0
            );
        }

        state.authorization = authorization.clone();

        queue_server_event_for_request(
            &mut state,
            None,
            ServerEvent::Session(SessionEvent::AuthorizationChanged { authorization }),
        );
    }

    pub(crate) fn refresh_semantic_sync_status(&self) {
        let Ok(mut state) = self.state.lock() else {
            error!("[viewport-data-channel] application session state is poisoned");
            return;
        };
        if !state.handshaken {
            return;
        }
        let status = state
            .interface
            .semantic_sync_status(&state.session_id)
            .unwrap_or_else(SemanticSyncStatus::disabled);
        if state.semantic_sync_status.as_ref() == Some(&status) {
            return;
        }
        state.semantic_sync_status = Some(status.clone());
        queue_server_event_for_request(
            &mut state,
            None,
            ServerEvent::Session(SessionEvent::SemanticSyncStatus { status }),
        );
    }

    pub(crate) fn flush_authoritative_events(&self, channel: &WebRTCDataChannel) {
        let Ok(mut state) = self.state.lock() else {
            error!("[viewport-data-channel] application session state is poisoned");
            return;
        };
        if !state.handshaken {
            return;
        }

        flush_pending_server_events(channel, &mut state);
    }
}

pub(super) fn remember_request_id(state: &mut ApplicationSessionState, request_id: String) -> bool {
    if state
        .recent_request_ids
        .iter()
        .any(|seen| seen == &request_id)
    {
        return false;
    }
    if state.recent_request_ids.len() >= MAX_RECENT_REQUEST_IDS {
        state.recent_request_ids.pop_front();
    }
    state.recent_request_ids.push_back(request_id);
    true
}

pub(super) fn admission_rejection_for(error: SessionAdmissionError) -> HandshakeRejectionReason {
    match error {
        SessionAdmissionError::ControllerAlreadyAssigned => {
            HandshakeRejectionReason::ControllerAlreadyAssigned
        }
        SessionAdmissionError::SessionAlreadyRegistered => {
            HandshakeRejectionReason::InvalidClientIdentity
        }
    }
}
