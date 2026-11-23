use std::path::Path;

use project_protocol::ProjectWriteErrorCode;
use usd_project::{ProjectId, SceneId};

use super::live_stage_runtime::{
    ActiveProjectRuntimeLease, ProjectRuntimeAuthorityRuntime, RUNTIME_LEASE_GRACE_MS,
    runtime_failed,
};
use crate::project::runtime::authority::ProjectRuntimeResponse;
use crate::project::service::ProjectRuntimeAuthorityQueue;

pub(crate) fn runtime_snapshot_response(
    world: &mut bevy::prelude::World,
    request_id: &str,
    project_id: ProjectId,
    active_scene_id: SceneId,
    queue: &ProjectRuntimeAuthorityQueue,
    project_root: &Path,
) -> ProjectRuntimeResponse {
    runtime_snapshot_response_inner(
        world,
        request_id,
        project_id,
        active_scene_id,
        queue,
        project_root,
        None,
    )
}

fn runtime_snapshot_response_inner(
    world: &mut bevy::prelude::World,
    request_id: &str,
    project_id: ProjectId,
    active_scene_id: SceneId,
    queue: &ProjectRuntimeAuthorityQueue,
    project_root: &Path,
    before_claim: Option<&dyn Fn()>,
) -> ProjectRuntimeResponse {
    if world
        .resource::<ProjectRuntimeAuthorityRuntime>()
        .active_lease
        .is_some()
    {
        return runtime_failed(request_id, ProjectWriteErrorCode::Busy);
    }
    if queue.is_cancelled(project_root, request_id) {
        return runtime_failed(request_id, ProjectWriteErrorCode::Busy);
    }
    let (root_layer, lease_id, session_id, live_revision, opened_root_digest) = {
        let Some(live) = world.get_non_send::<usd_bevy::LiveStage>() else {
            return runtime_failed(request_id, ProjectWriteErrorCode::Busy);
        };
        if queue.is_cancelled(project_root, request_id) {
            return runtime_failed(request_id, ProjectWriteErrorCode::Busy);
        }
        if !live.try_freeze_authoring() {
            return runtime_failed(request_id, ProjectWriteErrorCode::Busy);
        }
        if queue.is_cancelled(project_root, request_id) {
            live.unfreeze_authoring();
            return runtime_failed(request_id, ProjectWriteErrorCode::Busy);
        }
        let Ok(root_layer) = live.stage.root_layer().export_to_string() else {
            live.unfreeze_authoring();
            return runtime_failed(request_id, ProjectWriteErrorCode::ExportFailed);
        };
        if queue.is_cancelled(project_root, request_id) {
            live.unfreeze_authoring();
            return runtime_failed(request_id, ProjectWriteErrorCode::Busy);
        }
        (
            root_layer,
            uuid::Uuid::new_v4().to_string(),
            live.session_id(),
            live.authoring_generation().0,
            live.opened_root_bytes()
                .map(|bytes| *blake3::hash(bytes.as_ref()).as_bytes()),
        )
    };
    if let Some(before_claim) = before_claim {
        before_claim();
    }
    match queue.claim_request(project_root, request_id) {
        Ok(true) => {}
        Ok(false) | Err(_) => {
            if let Some(live) = world.get_non_send::<usd_bevy::LiveStage>() {
                live.unfreeze_authoring();
            }
            return runtime_failed(request_id, ProjectWriteErrorCode::Busy);
        }
    }
    world
        .resource_mut::<ProjectRuntimeAuthorityRuntime>()
        .active_lease = Some(ActiveProjectRuntimeLease {
        project_id,
        lease_id: lease_id.clone(),
        session_id,
        live_revision,
        expires_at_ms: crate::project::runtime::authority::unix_time_ms()
            .saturating_add(RUNTIME_LEASE_GRACE_MS),
    });
    ProjectRuntimeResponse::Ready {
        request_id: request_id.to_owned(),
        lease_id,
        session_id,
        scene_id: active_scene_id,
        live_revision,
        opened_root_digest,
        root_layer: root_layer.into_bytes(),
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/stage/project/runtime_snapshot_test_support.rs"]
pub(super) mod test_support;
