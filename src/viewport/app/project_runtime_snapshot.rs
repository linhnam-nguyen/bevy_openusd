use std::path::Path;

use project_protocol::ProjectWriteErrorCode;
use usd_project::{ProjectId, SceneId};

use crate::project::service::{ProjectRuntimeAuthorityQueue, ProjectRuntimeResponse};

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

#[cfg(test)]
pub(crate) fn runtime_snapshot_response_with_claim_hook(
    world: &mut bevy::prelude::World,
    request_id: &str,
    project_id: ProjectId,
    active_scene_id: SceneId,
    queue: &ProjectRuntimeAuthorityQueue,
    project_root: &Path,
    before_claim: &dyn Fn(),
) -> ProjectRuntimeResponse {
    runtime_snapshot_response_inner(
        world,
        request_id,
        project_id,
        active_scene_id,
        queue,
        project_root,
        Some(before_claim),
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
        .resource::<super::ProjectRuntimeAuthorityRuntime>()
        .active_lease
        .is_some()
    {
        return super::runtime_failed(request_id, ProjectWriteErrorCode::Busy);
    }
    if queue.is_cancelled(project_root, request_id) {
        return super::runtime_failed(request_id, ProjectWriteErrorCode::Busy);
    }
    let (root_layer, lease_id, session_id, live_revision) = {
        let Some(live) = world.get_non_send::<usd_bevy::LiveStage>() else {
            return super::runtime_failed(request_id, ProjectWriteErrorCode::Busy);
        };
        if queue.is_cancelled(project_root, request_id) {
            return super::runtime_failed(request_id, ProjectWriteErrorCode::Busy);
        }
        if !live.try_freeze_authoring() {
            return super::runtime_failed(request_id, ProjectWriteErrorCode::Busy);
        }
        if queue.is_cancelled(project_root, request_id) {
            live.unfreeze_authoring();
            return super::runtime_failed(request_id, ProjectWriteErrorCode::Busy);
        }
        let Ok(root_layer) = live.stage.root_layer().export_to_string() else {
            live.unfreeze_authoring();
            return super::runtime_failed(request_id, ProjectWriteErrorCode::ExportFailed);
        };
        if queue.is_cancelled(project_root, request_id) {
            live.unfreeze_authoring();
            return super::runtime_failed(request_id, ProjectWriteErrorCode::Busy);
        }
        (
            root_layer,
            uuid::Uuid::new_v4().to_string(),
            live.session_id(),
            live.authoring_generation().0,
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
            return super::runtime_failed(request_id, ProjectWriteErrorCode::Busy);
        }
    }
    world
        .resource_mut::<super::ProjectRuntimeAuthorityRuntime>()
        .active_lease = Some(super::ActiveProjectRuntimeLease {
        project_id,
        lease_id: lease_id.clone(),
        session_id,
        live_revision,
        expires_at_ms: crate::project::service::unix_time_ms()
            .saturating_add(super::RUNTIME_LEASE_GRACE_MS),
    });
    ProjectRuntimeResponse::Ready {
        request_id: request_id.to_owned(),
        lease_id,
        session_id,
        scene_id: active_scene_id,
        live_revision,
        root_layer: root_layer.into_bytes(),
    }
}
