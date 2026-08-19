use gstreamer_webrtc::WebRTCDataChannel;
use log::warn;
use viewport_protocol::{
    ClientCommand, ClientCommandEnvelope, CommandFamily, HandshakeEvent, HandshakeRejectionReason,
    PROTOCOL_VERSION, SemanticSyncStatus, ServerEvent, SessionEvent, SessionRole, StreamEvent,
};

use super::send::{rejection_for, send_handshake_rejection, send_server_event};
use crate::data_channel::session::{
    ApplicationSessionState, admission_rejection_for, remember_request_id,
};

pub(super) fn handle_handshake(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    envelope: ClientCommandEnvelope,
) {
    let request_id = envelope.request_id.clone();
    let ClientCommand::Handshake(hello) = envelope.command else {
        send_handshake_rejection(
            channel,
            state,
            HandshakeRejectionReason::InvalidClientIdentity,
        );
        return;
    };

    if envelope.session_id.is_some() {
        send_handshake_rejection(
            channel,
            state,
            HandshakeRejectionReason::InvalidClientIdentity,
        );
        return;
    }

    if let Err(error) = hello.validate() {
        send_handshake_rejection(channel, state, rejection_for(error));
        return;
    }
    if !hello
        .capabilities
        .protocol_versions
        .contains(&PROTOCOL_VERSION)
    {
        send_handshake_rejection(
            channel,
            state,
            HandshakeRejectionReason::UnsupportedProtocolVersion,
        );
        return;
    }
    if !hello
        .capabilities
        .command_families
        .contains(&CommandFamily::Session)
    {
        send_handshake_rejection(
            channel,
            state,
            HandshakeRejectionReason::UnsupportedCapabilities,
        );
        return;
    }

    let requested_role = hello.requested_role;
    state.client_name = hello.client_id.clone();
    if let Err(error) = state
        .admission
        .register(state.session_id.clone(), requested_role)
    {
        send_handshake_rejection(channel, state, admission_rejection_for(error));
        return;
    }
    state.role = Some(requested_role);

    let initial_metrics = if requested_role == SessionRole::Controller {
        let requested_metrics = state
            .server_capabilities
            .stream_limits
            .normalize(&hello.initial_viewport);
        match state
            .interface
            .submit_stream_configuration(requested_metrics)
        {
            Ok(metrics) => {
                state.pending_stream_configuration = Some(metrics.clone());
                state.latest_stream_generation = metrics.generation;
                Some(metrics)
            }
            Err(error) => {
                warn!("[viewport-data-channel] initial stream configuration rejected: {error:?}");
                state.admission.unregister(&state.session_id);
                state.role = None;
                send_handshake_rejection(
                    channel,
                    state,
                    HandshakeRejectionReason::UnsupportedCapabilities,
                );
                return;
            }
        }
    } else {
        None
    };

    state.client_sequence = envelope.sequence;
    remember_request_id(state, request_id);
    state.handshaken = true;
    let session_id = state.session_id.clone();
    let capabilities = state.server_capabilities.clone();
    let authorization = state.authorization.clone();
    let snapshot = state
        .interface
        .take_latest_snapshot(state.initial_snapshot.clone());
    send_server_event(
        channel,
        state,
        ServerEvent::Handshake(HandshakeEvent::ServerHello(
            viewport_protocol::ServerHello::with_authorization(
                session_id.clone(),
                hello.requested_role,
                capabilities,
                authorization,
            ),
        )),
    );
    if let Some(initial_metrics) = initial_metrics {
        send_server_event(
            channel,
            state,
            ServerEvent::Stream(StreamEvent::ConfigurationAccepted {
                metrics: initial_metrics,
            }),
        );
    }
    send_server_event(
        channel,
        state,
        ServerEvent::Session(SessionEvent::Ready {
            snapshot_required: true,
        }),
    );
    send_server_event(
        channel,
        state,
        ServerEvent::Session(SessionEvent::Snapshot { state: snapshot }),
    );
    let semantic_sync_status = state
        .interface
        .semantic_sync_status(&session_id)
        .unwrap_or_else(SemanticSyncStatus::disabled);
    state.semantic_sync_status = Some(semantic_sync_status.clone());
    send_server_event(
        channel,
        state,
        ServerEvent::Session(SessionEvent::SemanticSyncStatus {
            status: semantic_sync_status,
        }),
    );
}
