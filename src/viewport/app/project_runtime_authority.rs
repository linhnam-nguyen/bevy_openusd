//! Render-owner request handling for Project Commit and Export authority.

use bevy::prelude::World;

use crate::{project::catalog::manifest_store::ManifestStore, viewport::session::StageHandle};

use super::{
    ProjectRuntimeAuthorityRuntime, active_scene_id_for_stage, project_root_for_stage,
    runtime_finish_response, runtime_revision_response, runtime_scene_is_allowed,
    runtime_scene_is_in_export_closure, runtime_snapshot_response,
};
use crate::project::service::{ProjectRuntimeRequest, ProjectRuntimeResponse};

/// Answer host-side Commit/Export requests from the exact LiveStage owner.
pub(super) fn consume_project_runtime_authority(world: &mut World) {
    let Some(stage_path) = world
        .get_resource::<StageHandle>()
        .map(|handle| handle.path.clone())
    else {
        return;
    };
    let Some(project_root) = project_root_for_stage(&stage_path) else {
        return;
    };
    let Ok(manifest) = ManifestStore::read_validated(&project_root) else {
        return;
    };
    let project_id = manifest.raw().project_id;
    let Some(active_scene_id) = active_scene_id_for_stage(&stage_path, &project_root) else {
        return;
    };
    let queue = world
        .resource::<ProjectRuntimeAuthorityRuntime>()
        .queue
        .clone();
    let Ok(requests) = queue.consume_pending(&project_root) else {
        return;
    };
    for request in requests {
        let request_id = request.request_id().to_owned();
        let response = match request {
            ProjectRuntimeRequest::BeginCommit {
                project_id: request_project_id,
                target,
                ..
            } if request_project_id == project_id => {
                let allowed =
                    runtime_scene_is_allowed(&project_root, &manifest, active_scene_id, &target);
                if !allowed {
                    ProjectRuntimeResponse::Inactive { request_id }
                } else {
                    runtime_snapshot_response(world, &request_id, project_id, active_scene_id)
                }
            }
            ProjectRuntimeRequest::ExportScene {
                project_id: request_project_id,
                scene_id,
                ..
            } if request_project_id == project_id => {
                if runtime_scene_is_in_export_closure(
                    &project_root,
                    &manifest,
                    scene_id,
                    active_scene_id,
                ) {
                    runtime_snapshot_response(world, &request_id, project_id, active_scene_id)
                } else {
                    ProjectRuntimeResponse::Inactive { request_id }
                }
            }
            ProjectRuntimeRequest::ValidateCommit {
                project_id: request_project_id,
                lease_id,
                live_revision,
                ..
            } if request_project_id == project_id => {
                runtime_revision_response(world, &request_id, project_id, &lease_id, live_revision)
            }
            ProjectRuntimeRequest::FinishCommit {
                project_id: request_project_id,
                lease_id,
                live_revision,
                ..
            } if request_project_id == project_id => {
                runtime_finish_response(world, &request_id, project_id, &lease_id, live_revision)
            }
            ProjectRuntimeRequest::AbortCommit {
                project_id: request_project_id,
                lease_id,
                ..
            } if request_project_id == project_id => {
                super::runtime_abort_response(world, &request_id, project_id, &lease_id)
            }
            _ => ProjectRuntimeResponse::Inactive { request_id },
        };
        if let Err(error) = queue.publish_response(&project_root, &response) {
            if let ProjectRuntimeResponse::Ready { lease_id, .. } = &response {
                super::clear_runtime_lease(world, project_id, lease_id);
            }
            bevy::log::warn!("Project runtime authority response failed: {error:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_lease_rejects_a_synchronous_authoring_generation_change() {
        let stage = openusd::usd::Stage::builder()
            .in_memory("runtime-lease-generation.usda")
            .expect("in-memory stage");
        let live = usd_bevy::LiveStage::new(stage);
        let project_id = usd_project::ProjectId::new_v4();
        let session_id = live.session_id();
        let mut world = World::new();
        world.insert_resource(ProjectRuntimeAuthorityRuntime {
            queue: crate::project::service::ProjectRuntimeAuthorityQueue::default(),
            active_lease: Some(super::super::ActiveProjectRuntimeLease {
                project_id,
                lease_id: "lease".to_owned(),
                session_id,
                live_revision: 0,
            }),
        });
        world.insert_non_send(live);
        let live = world
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("live stage");
        assert!(live.try_freeze_authoring());
        usd_bevy::authoring::define_prim(&live.stage, "/ConcurrentEdit", "Xform")
            .expect("author concurrent edit");
        assert!(live.authoring_generation().0 > 0);

        assert!(matches!(
            super::super::runtime_revision_response(&world, "request", project_id, "lease", 0),
            ProjectRuntimeResponse::Failed {
                code: project_protocol::ProjectWriteErrorCode::ConcurrentChange,
                ..
            }
        ));
    }
}
