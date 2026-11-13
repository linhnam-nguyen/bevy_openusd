use bevy::prelude::*;
use viewport_protocol::{SemanticSyncPhase, SemanticSyncStatus};

use crate::project::semantic_store::sync::{
    TursoClientSyncProvisionRequest, TursoClientSyncRuntime, TursoClientSyncRuntimeCommand,
    TursoClientSyncRuntimeSubmitError,
};
use crate::viewport::api::RenderServerInterface;
use crate::viewport::semantic::SemanticSyncState;

#[derive(Resource, Default)]
pub(super) struct SemanticSyncRuntimeResource(pub(super) Option<TursoClientSyncRuntime>);

pub(super) fn convert_semantic_sync_request(
    request: viewport_streaming::SemanticSyncRequest,
    snapshot: Option<&usd_model::SemanticSnapshot>,
) -> Option<(TursoClientSyncRuntimeCommand, bool)> {
    let session_id = request.session_id;
    let client_name = request.client_name;
    let authorization = request.authorization;

    let (command, is_close) = match request.kind {
        viewport_streaming::SemanticSyncRequestKind::Client(operation) => {
            let is_close = operation == viewport_protocol::SemanticSyncOperation::Close;

            let command = match operation {
                viewport_protocol::SemanticSyncOperation::Provision => {
                    TursoClientSyncRuntimeCommand::Provision(TursoClientSyncProvisionRequest {
                        session_id: session_id.clone(),
                        client_name,
                        authorization,
                    })
                }
                viewport_protocol::SemanticSyncOperation::Connect => {
                    TursoClientSyncRuntimeCommand::Connect(session_id.clone())
                }
                viewport_protocol::SemanticSyncOperation::PushSnapshot => {
                    let snapshot = snapshot?.clone();
                    TursoClientSyncRuntimeCommand::PushSnapshot {
                        session_id: session_id.clone(),
                        snapshot,
                    }
                }
                viewport_protocol::SemanticSyncOperation::PullProjection => {
                    TursoClientSyncRuntimeCommand::PullProjection(session_id.clone())
                }
                viewport_protocol::SemanticSyncOperation::Close => {
                    TursoClientSyncRuntimeCommand::Close(session_id.clone())
                }
            };

            (command, is_close)
        }

        viewport_streaming::SemanticSyncRequestKind::AuthorizationChanged => (
            TursoClientSyncRuntimeCommand::UpdateAuthorization {
                session_id: session_id.clone(),
                authorization,
            },
            false,
        ),
    };

    Some((command, is_close))
}

pub(super) fn process_semantic_sync_requests(
    interface: Res<RenderServerInterface>,
    runtime: Res<SemanticSyncRuntimeResource>,
    semantic: Res<SemanticSyncState>,
) {
    let application_interface = interface.shared();
    while let Some(request) = application_interface.pop_semantic_sync_request() {
        let request_id = request.request_id.clone();
        let session_id = request.session_id.clone();

        let Some((command, is_close)) = convert_semantic_sync_request(request, semantic.snapshot())
        else {
            let _ = application_interface.publish_semantic_sync_status(
                session_id.clone(),
                SemanticSyncStatus::phase(
                    SemanticSyncPhase::Failed,
                    Some("snapshot_unavailable".to_owned()),
                ),
            );
            continue;
        };

        let Some(runtime) = runtime.0.as_ref() else {
            let status = if is_close {
                SemanticSyncStatus::phase(SemanticSyncPhase::Closed, None)
            } else {
                SemanticSyncStatus::phase(
                    SemanticSyncPhase::Failed,
                    Some("runtime_unavailable".to_owned()),
                )
            };
            let _ = application_interface.publish_semantic_sync_status(session_id, status);
            continue;
        };
        if let Err(error) = runtime.submit(command) {
            bevy::log::warn!(
                "[semantic-sync] request {} could not reach worker: {error:#}",
                request_id
            );
            let detail = match error {
                TursoClientSyncRuntimeSubmitError::QueueFull => "runtime_queue_full",
                TursoClientSyncRuntimeSubmitError::WorkerUnavailable => "worker_unavailable",
                TursoClientSyncRuntimeSubmitError::SessionClosed => "session_closed",
            };
            let _ = application_interface.publish_semantic_sync_status(
                session_id,
                SemanticSyncStatus::phase(SemanticSyncPhase::Failed, Some(detail.to_owned())),
            );
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/app/sync_inline_tests.rs"]
mod tests;
