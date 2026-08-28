use gstreamer_webrtc::WebRTCDataChannel;
use log::warn;
use project_protocol::ProjectActivationReply;
use viewport_protocol::{
    HandshakeEvent, HandshakeRejectionReason, ProtocolValidationError, ServerEvent,
    ServerEventEnvelope, StreamEvent, ViewportEvent, encode_server_json_line,
};

use crate::data_channel::events::queue_server_event_for_request;
use crate::data_channel::session::ApplicationSessionState;

pub(crate) fn rejection_for(error: ProtocolValidationError) -> HandshakeRejectionReason {
    match error {
        ProtocolValidationError::UnsupportedProtocolVersion { .. } => {
            HandshakeRejectionReason::UnsupportedProtocolVersion
        }
        ProtocolValidationError::EmptyField { .. } => {
            HandshakeRejectionReason::InvalidClientIdentity
        }
        _ => HandshakeRejectionReason::UnsupportedCapabilities,
    }
}

pub(crate) fn send_handshake_rejection(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    reason: HandshakeRejectionReason,
) {
    send_server_event(
        channel,
        state,
        ServerEvent::Handshake(HandshakeEvent::Rejected { reason }),
    );
}

pub(crate) fn send_server_event(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    event: ServerEvent,
) {
    send_server_event_with_request(channel, state, None, event);
}

pub(crate) fn send_server_event_for_request(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    request_id: String,
    event: ServerEvent,
) {
    send_server_event_with_request(channel, state, Some(request_id), event);
}

pub(crate) fn send_command_rejection(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    request_id: String,
    reason: String,
) {
    send_server_event_for_request(
        channel,
        state,
        request_id.clone(),
        ServerEvent::Viewport(ViewportEvent::CommandRejected { request_id, reason }),
    );
}

pub(crate) fn send_stream_configuration_rejection(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    request_id: String,
    reason: String,
) {
    send_server_event_for_request(
        channel,
        state,
        request_id,
        ServerEvent::Stream(StreamEvent::ConfigurationRejected { reason }),
    );
}

pub(crate) fn send_runtime_blob_rejection(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    request_id: String,
    reason: String,
) {
    send_server_event_for_request(
        channel,
        state,
        request_id,
        ServerEvent::Session(viewport_protocol::SessionEvent::RuntimeBlobRejected { reason }),
    );
}

pub(crate) fn send_server_event_with_request(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    request_id: Option<String>,
    event: ServerEvent,
) {
    queue_server_event_for_request(state, request_id, event);
    flush_pending_server_events(channel, state);
}

pub(crate) fn next_server_envelope(
    state: &mut ApplicationSessionState,
    request_id: Option<&str>,
    event: ServerEvent,
) -> ServerEventEnvelope {
    let sequence = state.server_sequence;
    state.server_sequence = state.server_sequence.saturating_add(1);
    match request_id {
        Some(request_id) => {
            let mut envelope = ServerEventEnvelope::for_request(
                state.session_id.clone(),
                sequence,
                request_id,
                event,
            );
            envelope.causation_id = Some(request_id.to_owned());
            envelope
        }
        None => ServerEventEnvelope::new(state.session_id.clone(), sequence, event),
    }
}

pub(crate) fn encoded_size(envelope: &ServerEventEnvelope) -> Option<usize> {
    encode_server_json_line(envelope)
        .ok()
        .map(|json| json.len())
}

pub(crate) fn flush_pending_server_events(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
) {
    while let Some(envelope) = state.pending_server_events.front().cloned() {
        let result = encode_server_json_line(&envelope)
            .map_err(|error| error.to_string())
            .and_then(|json| {
                channel
                    .send_string_full(Some(&json))
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(()) => {
                state.pending_server_events.pop_front();
            }
            Err(error) => {
                warn!("[viewport-data-channel] application event send failed: {error}");
                if error.to_ascii_lowercase().contains("too large") {
                    if let Some(oversized) = state.pending_server_events.pop_front() {
                        let rejection_id = oversized
                            .request_id
                            .clone()
                            .unwrap_or_else(|| format!("server-sequence-{}", oversized.sequence));
                        let mut replacement = oversized;
                        replacement.event = ServerEvent::Viewport(ViewportEvent::CommandRejected {
                            request_id: rejection_id,
                            reason: "application event exceeded the DataChannel message limit"
                                .to_owned(),
                        });
                        state.pending_server_events.push_front(replacement);
                    }
                    continue;
                }
                break;
            }
        }
    }
}

/// Sends a Project activation result on the reliable control channel. This is
/// deliberately a Project-protocol frame rather than a viewport-protocol
/// event, keeping Project management out of the viewport contract.
pub(crate) fn send_project_activation_reply(
    channel: &WebRTCDataChannel,
    reply: &ProjectActivationReply,
) {
    let result = serde_json::to_string(reply)
        .map_err(|error| error.to_string())
        .and_then(|json| {
            channel
                .send_string_full(Some(&json))
                .map_err(|error| error.to_string())
        });
    if let Err(error) = result {
        warn!("[viewport-data-channel] Project activation reply send failed: {error}");
    }
}
