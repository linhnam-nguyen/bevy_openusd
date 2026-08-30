//! Render-owner request handling for Project Commit and Export authority.

use bevy::prelude::World;

use crate::{project::catalog::manifest_store::ManifestStore, viewport::session::StageHandle};

use super::{
    ProjectRuntimeAuthorityRuntime, active_scene_id_for_stage, project_root_for_stage,
    runtime_scene_is_allowed, runtime_scene_is_in_export_closure,
};
use crate::project::service::{ProjectRuntimeRequest, ProjectRuntimeResponse};

#[cfg(test)]
#[path = "project_runtime_authority_owner_review5_tests.rs"]
mod owner_review5_tests;

/// Answer host-side Commit/Export requests from the exact LiveStage owner.
pub(super) fn consume_project_runtime_authority(world: &mut World) {
    super::expire_runtime_lease(world);
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
    let active = world.get_resource::<StageHandle>().and_then(|handle| {
        let project_root = project_root_for_stage(&handle.path)?;
        let manifest = ManifestStore::read_validated(&project_root).ok()?;
        let project_id = manifest.raw().project_id;
        let active_scene_id = active_scene_id_for_stage(&handle.path, &project_root);
        Some((project_root, project_id, active_scene_id, manifest))
    });
    let active_root = active.as_ref().map(|(root, ..)| root);
    for envelope in requests {
        let expired = envelope.is_expired(crate::project::service::unix_time_ms());
        let request = envelope.into_request();
        let request_id = request.request_id().to_owned();
        let registered_project_id = request.project_id();
        let Some(project_root) = registered_project_roots
            .get(&registered_project_id)
            .cloned()
        else {
            let response =
                super::runtime_failed(&request_id, project_protocol::ProjectWriteErrorCode::Busy);
            if let Err(error) = queue.publish_response(&response) {
                bevy::log::warn!("Project runtime authority response failed: {error:?}");
            }
            queue.clear_request_state(std::path::Path::new("."), &request_id);
            continue;
        };
        let cancelled = queue.is_cancelled(&project_root, &request_id);
        let active_request = active
            .as_ref()
            .and_then(|(_, project_id, scene_id, manifest)| {
                (*scene_id)
                    .filter(|_| {
                        request.project_id() == registered_project_id
                            && registered_project_id == *project_id
                            && active_root.is_some_and(|root| root == &project_root)
                    })
                    .map(|active_scene_id| (project_id, active_scene_id, manifest))
            });
        let response = if expired || cancelled {
            super::runtime_failed(&request_id, project_protocol::ProjectWriteErrorCode::Busy)
        } else if let Some((project_id, active_scene_id, manifest)) = active_request {
            match request {
                ProjectRuntimeRequest::BeginCommit { target, .. } => {
                    if runtime_scene_is_allowed(&project_root, manifest, active_scene_id, &target) {
                        super::runtime_snapshot_response(
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
                        super::runtime_snapshot_response(
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
                } => super::runtime_revision_response(
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
                } => super::runtime_finish_response(
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
                } => super::runtime_renew_response(
                    world,
                    &request_id,
                    *project_id,
                    &lease_id,
                    live_revision,
                ),
                ProjectRuntimeRequest::AbortCommit { lease_id, .. } => {
                    super::runtime_abort_response(world, &request_id, *project_id, &lease_id)
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
                if let Some((_, project_id, ..)) = active.as_ref() {
                    super::clear_runtime_lease(world, *project_id, lease_id);
                }
            }
            queue.clear_request_state(&project_root, &request_id);
            bevy::log::warn!("Project runtime authority response failed: {error:?}");
        } else if !response_is_ready {
            queue.clear_request_state(&project_root, &request_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, thread, time::Duration};

    use crate::project::service::ProjectRuntimeAuthority;
    use project_protocol::ProjectCommitTarget;

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
                expires_at_ms: crate::project::service::unix_time_ms() + 60_000,
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

    #[test]
    fn no_active_stage_answers_registered_requests_as_inactive() {
        let directory = tempfile::tempdir().expect("temporary runtime root");
        let project_id = usd_project::ProjectId::new_v4();
        let registry_path = directory.path().join("workspace.json");
        let mut registry =
            crate::project::catalog::workspace_registry::WorkspaceRegistry::load(&registry_path)
                .expect("workspace registry");
        registry
            .register(project_id, directory.path(), None)
            .expect("register project root");
        let queue = Arc::new(
            crate::project::service::ProjectRuntimeAuthorityQueue::with_timeout_and_registry_path(
                Duration::from_millis(250),
                registry_path,
            ),
        );
        let request_queue = queue.clone();
        let root = directory.path().to_path_buf();
        let caller = thread::spawn(move || {
            request_queue.begin_commit(&root, project_id, &ProjectCommitTarget::Project)
        });
        let request_directory = directory
            .path()
            .join(".usdhub/cache/project-runtime-authority/requests");
        for _ in 0..100 {
            if request_directory.is_dir()
                && request_directory
                    .read_dir()
                    .expect("request directory")
                    .filter_map(Result::ok)
                    .any(|entry| {
                        entry.path().extension().and_then(|value| value.to_str()) == Some("json")
                    })
            {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }

        let mut world = World::new();
        world.insert_resource(ProjectRuntimeAuthorityRuntime {
            queue: (*queue).clone(),
            active_lease: None,
        });
        consume_project_runtime_authority(&mut world);

        assert!(matches!(caller.join().expect("authority caller"), Ok(None)));
    }

    #[test]
    fn expired_runtime_lease_unfreezes_live_stage() {
        let stage = openusd::usd::Stage::builder()
            .in_memory("expired-runtime-lease.usda")
            .expect("in-memory stage");
        let live = usd_bevy::LiveStage::new(stage);
        let project_id = usd_project::ProjectId::new_v4();
        let session_id = live.session_id();
        let mut world = World::new();
        world.insert_resource(ProjectRuntimeAuthorityRuntime {
            queue: crate::project::service::ProjectRuntimeAuthorityQueue::default(),
            active_lease: Some(super::super::ActiveProjectRuntimeLease {
                project_id,
                lease_id: "expired".to_owned(),
                session_id,
                live_revision: 0,
                expires_at_ms: 0,
            }),
        });
        world.insert_non_send(live);
        world
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("live stage")
            .try_freeze_authoring();

        super::super::expire_runtime_lease(&mut world);

        assert!(
            !world
                .get_non_send::<usd_bevy::LiveStage>()
                .expect("live stage")
                .is_authoring_frozen()
        );
        assert!(
            world
                .resource::<ProjectRuntimeAuthorityRuntime>()
                .active_lease
                .is_none()
        );
    }
}
