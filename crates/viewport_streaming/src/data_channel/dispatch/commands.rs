use gstreamer_webrtc::WebRTCDataChannel;
use viewport_protocol::{
    ClientCommand, ClientCommandEnvelope, SemanticSyncOperation, ServerEvent, SessionCommand,
    SessionEvent, SessionRole, StreamCommand, StreamEvent, ViewportCommandEnvelope,
};

use super::send::{
    flush_pending_server_events, send_command_rejection, send_runtime_blob_rejection,
    send_server_event_for_request, send_stream_configuration_rejection,
};
use crate::application::{SemanticSyncRequest, SemanticSyncRequestKind};
use crate::data_channel::chunks::queue_runtime_blob;
use crate::data_channel::session::ApplicationSessionState;

pub(super) fn handle_authenticated_command(
    channel: &WebRTCDataChannel,
    state: &mut ApplicationSessionState,
    envelope: ClientCommandEnvelope,
) {
    let request_id = envelope.request_id;
    match envelope.command {
        ClientCommand::Viewport(command) if state.role != Some(SessionRole::Controller) => {
            send_command_rejection(
                channel,
                state,
                request_id,
                "observer sessions are read-only".to_owned(),
            );
            let _ = command;
        }
        ClientCommand::Viewport(command) => {
            let viewport_command = ViewportCommandEnvelope {
                protocol_version: envelope.protocol_version,
                request_id: request_id.clone(),
                command,
            };
            if let Err(error) = state.interface.submit_viewport_command(viewport_command) {
                send_command_rejection(
                    channel,
                    state,
                    request_id,
                    format!("viewport command rejected: {error:?}"),
                );
            }
        }
        ClientCommand::Input(command) if state.role != Some(SessionRole::Controller) => {
            send_command_rejection(
                channel,
                state,
                request_id,
                "observer sessions cannot control the viewport".to_owned(),
            );
            let _ = command;
        }
        ClientCommand::Input(command) => {
            if let Err(error) = state.interface.submit_input(command) {
                send_command_rejection(
                    channel,
                    state,
                    request_id,
                    format!("input command rejected: {error:?}"),
                );
            }
        }
        ClientCommand::Session(SessionCommand::RequestSnapshot) => {
            let snapshot = state
                .interface
                .take_latest_snapshot(state.initial_snapshot.clone());
            send_server_event_for_request(
                channel,
                state,
                request_id,
                ServerEvent::Session(SessionEvent::Snapshot { state: snapshot }),
            );
        }
        ClientCommand::Session(SessionCommand::SemanticSync { operation }) => {
            if operation != SemanticSyncOperation::Close
                && (!state.authorization.allows_self_render_delivery()
                    || !state.authorization.allows_model_download())
            {
                send_command_rejection(
                    channel,
                    state,
                    request_id,
                    "semantic synchronization is not authorized for this session".to_owned(),
                );
                return;
            }
            let request = SemanticSyncRequest {
                request_id: request_id.clone(),
                session_id: state.session_id.clone(),
                client_name: state.client_name.clone(),
                authorization: state.authorization.clone(),
                kind: SemanticSyncRequestKind::Client(operation),
            };
            let submit_res = if operation == SemanticSyncOperation::Close {
                state
                    .interface
                    .submit_semantic_sync_control_request(request)
            } else {
                state.interface.submit_semantic_sync_request(request)
            };
            if let Err(error) = submit_res {
                send_command_rejection(
                    channel,
                    state,
                    request_id,
                    format!("semantic-sync request rejected: {error:?}"),
                );
            }
        }
        ClientCommand::Session(SessionCommand::RequestRuntimeManifest) => {
            let Some(manifest) = state.interface.runtime_manifest() else {
                send_runtime_blob_rejection(
                    channel,
                    state,
                    request_id,
                    "runtime manifest is not available".to_owned(),
                );
                return;
            };
            match manifest.authorize(&state.authorization) {
                Ok(manifest) => send_server_event_for_request(
                    channel,
                    state,
                    request_id,
                    ServerEvent::Session(SessionEvent::RuntimeManifest { manifest }),
                ),
                Err(error) => {
                    send_runtime_blob_rejection(channel, state, request_id, error.to_string())
                }
            }
        }
        ClientCommand::Session(SessionCommand::RequestRuntimeBlob { blob_id }) => {
            let Some(manifest) = state.interface.runtime_manifest() else {
                send_runtime_blob_rejection(
                    channel,
                    state,
                    request_id,
                    "runtime manifest is not available".to_owned(),
                );
                return;
            };
            let authorized = match manifest.authorize(&state.authorization) {
                Ok(manifest) => manifest,
                Err(error) => {
                    send_runtime_blob_rejection(channel, state, request_id, error.to_string());
                    return;
                }
            };
            if !authorized.allows_blob(&blob_id) {
                send_runtime_blob_rejection(
                    channel,
                    state,
                    request_id,
                    "requested runtime blob is not authorized by the manifest".to_owned(),
                );
                return;
            }
            let Some(bytes) = state.interface.runtime_blob(&blob_id) else {
                send_runtime_blob_rejection(
                    channel,
                    state,
                    request_id,
                    "requested runtime blob is not available".to_owned(),
                );
                return;
            };
            let expected_size = authorized
                .references()
                .into_iter()
                .find(|reference| reference.blob_id == blob_id)
                .map(|reference| reference.byte_size);
            if expected_size != Some(bytes.len() as u64) {
                send_runtime_blob_rejection(
                    channel,
                    state,
                    request_id,
                    "runtime blob byte size does not match its authorized manifest reference"
                        .to_owned(),
                );
                return;
            }
            queue_runtime_blob(state, Some(&request_id), blob_id, bytes);
            flush_pending_server_events(channel, state);
        }
        ClientCommand::Session(SessionCommand::Ping { nonce }) => {
            send_server_event_for_request(
                channel,
                state,
                request_id,
                ServerEvent::Session(SessionEvent::Pong { nonce }),
            );
        }
        ClientCommand::Stream(StreamCommand::ConfigureViewport { metrics })
            if state.role != Some(SessionRole::Controller) =>
        {
            send_stream_configuration_rejection(
                channel,
                state,
                request_id,
                "observer sessions cannot resize the shared stream".to_owned(),
            );
            let _ = metrics;
        }
        ClientCommand::Stream(StreamCommand::ConfigureViewport { metrics }) => {
            let requested_metrics = state.server_capabilities.stream_limits.normalize(&metrics);
            if requested_metrics.generation <= state.latest_stream_generation {
                let latest_generation = state.latest_stream_generation;
                send_stream_configuration_rejection(
                    channel,
                    state,
                    request_id,
                    format!(
                        "stream generation {} is not newer than active generation {}",
                        requested_metrics.generation, latest_generation
                    ),
                );
                return;
            }
            let metrics = match state
                .interface
                .submit_stream_configuration(requested_metrics)
            {
                Ok(metrics) => metrics,
                Err(error) => {
                    send_stream_configuration_rejection(
                        channel,
                        state,
                        request_id,
                        format!("stream configuration rejected: {error:?}"),
                    );
                    return;
                }
            };
            state.pending_stream_configuration = Some(metrics.clone());
            state.latest_stream_generation = metrics.generation;
            send_server_event_for_request(
                channel,
                state,
                request_id,
                ServerEvent::Stream(StreamEvent::ConfigurationAccepted { metrics }),
            );
        }
        command => {
            send_command_rejection(
                channel,
                state,
                request_id,
                format!("command family is not available in this phase: {command:?}"),
            );
        }
    }
}
