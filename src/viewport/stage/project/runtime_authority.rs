//! Render-owner request handling for Project Commit and Export authority.

use std::path::Path;

use bevy::prelude::World;
use usd_project::{SceneId, ValidatedProjectManifest};

use crate::{
    project::{catalog::manifest_store::ManifestStore, storage::ProjectStorageLayout},
    viewport::stage::StageHandle,
};

use super::live_stage_runtime::{
    ProjectRuntimeAuthorityRuntime, clear_runtime_lease, expire_runtime_lease,
    runtime_abort_response, runtime_failed, runtime_finish_response, runtime_renew_response,
    runtime_revision_response, runtime_scene_is_allowed, runtime_scene_is_in_export_closure,
    runtime_snapshot_response,
};
use crate::project::runtime::authority::{ProjectRuntimeRequest, ProjectRuntimeResponse};

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/stage/project/runtime_authority_lease_tests.rs"]
mod lease_tests;

/// Answer host-side Commit/Export requests from the exact LiveStage owner.
pub(super) fn consume_project_runtime_authority(world: &mut World) {
    expire_runtime_lease(world);
    let queue = world
        .resource::<ProjectRuntimeAuthorityRuntime>()
        .queue
        .clone();
    let Ok(requests) = queue.consume_pending() else {
        return;
    };
    if requests.is_empty() {
        return;
    }
    let registered_project_roots = queue.registered_project_roots();
    let active_path = world
        .get_resource::<StageHandle>()
        .filter(|handle| handle.error.is_none())
        .map(|handle| handle.path.clone());
    for envelope in requests {
        let expired = envelope.is_expired(crate::project::runtime::authority::unix_time_ms());
        let request = envelope.into_request();
        let request_id = request.request_id().to_owned();
        let registered_project_id = request.project_id();
        let Some(project_root) = registered_project_roots
            .get(&registered_project_id)
            .cloned()
        else {
            let response =
                runtime_failed(&request_id, project_protocol::ProjectWriteErrorCode::Busy);
            if let Err(error) = queue.publish_response(&response) {
                bevy::log::warn!("Project runtime authority response failed: {error:?}");
            }
            queue.clear_request_state(std::path::Path::new("."), &request_id);
            continue;
        };
        let cancelled = queue.is_cancelled(&project_root, &request_id);
        let active_request = active_path.as_deref().and_then(|stage_path| {
            let manifest = ManifestStore::read_validated(&project_root).ok()?;
            if manifest.raw().project_id != registered_project_id {
                return None;
            }
            let scene_id = manifest_scene_for_stage(stage_path, &project_root, &manifest)?;
            Some((scene_id, manifest))
        });
        let response = if expired || cancelled {
            runtime_failed(&request_id, project_protocol::ProjectWriteErrorCode::Busy)
        } else if let Some((active_scene_id, manifest)) = active_request {
            let project_id = &registered_project_id;
            let manifest = &manifest;
            match request {
                ProjectRuntimeRequest::BeginCommit { target, .. } => {
                    if runtime_scene_is_allowed(&project_root, manifest, active_scene_id, &target) {
                        runtime_snapshot_response(
                            world,
                            &request_id,
                            *project_id,
                            active_scene_id,
                            &queue,
                            &project_root,
                        )
                    } else {
                        ProjectRuntimeResponse::Inactive {
                            request_id: request_id.clone(),
                        }
                    }
                }
                ProjectRuntimeRequest::ExportScene { scene_id, .. } => {
                    if runtime_scene_is_in_export_closure(
                        &project_root,
                        manifest,
                        scene_id,
                        active_scene_id,
                    ) {
                        runtime_snapshot_response(
                            world,
                            &request_id,
                            *project_id,
                            active_scene_id,
                            &queue,
                            &project_root,
                        )
                    } else {
                        ProjectRuntimeResponse::Inactive {
                            request_id: request_id.clone(),
                        }
                    }
                }
                ProjectRuntimeRequest::ValidateCommit {
                    lease_id,
                    live_revision,
                    ..
                } => runtime_revision_response(
                    world,
                    &request_id,
                    *project_id,
                    &lease_id,
                    live_revision,
                ),
                ProjectRuntimeRequest::FinishCommit {
                    lease_id,
                    live_revision,
                    ..
                } => runtime_finish_response(
                    world,
                    &request_id,
                    *project_id,
                    &lease_id,
                    live_revision,
                ),
                ProjectRuntimeRequest::RenewCommitLease {
                    lease_id,
                    live_revision,
                    ..
                } => runtime_renew_response(
                    world,
                    &request_id,
                    *project_id,
                    &lease_id,
                    live_revision,
                ),
                ProjectRuntimeRequest::AbortCommit { lease_id, .. } => {
                    runtime_abort_response(world, &request_id, *project_id, &lease_id)
                }
            }
        } else {
            ProjectRuntimeResponse::Inactive {
                request_id: request_id.clone(),
            }
        };
        let response_is_ready = matches!(&response, ProjectRuntimeResponse::Ready { .. });
        if let Err(error) = queue.publish_response(&response) {
            if let ProjectRuntimeResponse::Ready { lease_id, .. } = &response {
                clear_runtime_lease(world, registered_project_id, lease_id);
            }
            queue.clear_request_state(&project_root, &request_id);
            bevy::log::warn!("Project runtime authority response failed: {error:?}");
        } else if !response_is_ready {
            queue.clear_request_state(&project_root, &request_id);
        }
    }
}

/// Match the open file against the registered Project's current Scene membership.
/// The storage layout is also used by Stage activation, including legacy paths.
fn manifest_scene_for_stage(
    stage_path: &Path,
    project_root: &Path,
    manifest: &ValidatedProjectManifest,
) -> Option<SceneId> {
    let stage_path = stage_path.canonicalize().ok()?;
    let root = project_root.canonicalize().ok()?;
    if !stage_path.starts_with(&root) {
        return None;
    }
    let layout = ProjectStorageLayout::new(root);
    let mut matches = manifest.raw().scenes.iter().filter(|scene| {
        layout
            .readable_scene_path(manifest.raw(), scene)
            .canonicalize()
            .is_ok_and(|candidate| candidate == stage_path)
    });
    let scene = matches.next()?;
    matches.next().is_none().then_some(scene.id)
}

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/stage/project/runtime_authority_tests.rs"]
mod tests;
