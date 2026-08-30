//! Render-server owner for the Project-to-LiveStage handoff.

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use project_protocol::ProjectCommitTarget;
use usd_project::{ProjectId, SceneId};

use crate::{
    project::{
        catalog::manifest_store::ManifestStore,
        service::{
            ProjectRuntimeAuthorityQueue, ProjectRuntimeRequest, ProjectRuntimeResponse,
            ProjectStageMutationQueue,
        },
    },
    viewport::session::StageHandle,
};

/// The render-server process owns this queue resource. Project mutation
/// records are read from the active Project's private cache outbox.
#[derive(Resource, Clone, Default)]
pub(super) struct ProjectStageMutationRuntime(pub(super) ProjectStageMutationQueue);

#[derive(Resource, Clone, Default)]
pub(super) struct ProjectRuntimeAuthorityRuntime(pub(super) ProjectRuntimeAuthorityQueue);

pub(super) fn install(app: &mut App) {
    app.insert_resource(ProjectStageMutationRuntime::default())
        .insert_resource(ProjectRuntimeAuthorityRuntime::default())
        .add_systems(
            Update,
            (
                consume_project_runtime_authority,
                consume_project_stage_mutations,
            )
                .in_set(usd_bevy::LiveStageSet::Project),
        );
}

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
    let queue = world.resource::<ProjectRuntimeAuthorityRuntime>().0.clone();
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
                let allowed = match target {
                    ProjectCommitTarget::Project => true,
                    ProjectCommitTarget::Scene(scene_id) => scene_id == active_scene_id,
                };
                if !allowed {
                    ProjectRuntimeResponse::Inactive { request_id }
                } else {
                    runtime_snapshot_response(world, &request_id, active_scene_id)
                }
            }
            ProjectRuntimeRequest::ExportScene {
                project_id: request_project_id,
                scene_id,
                ..
            } if request_project_id == project_id && scene_id == active_scene_id => {
                runtime_snapshot_response(world, &request_id, active_scene_id)
            }
            ProjectRuntimeRequest::ValidateCommit {
                project_id: request_project_id,
                live_revision,
                ..
            } if request_project_id == project_id => {
                runtime_revision_response(world, &request_id, live_revision)
            }
            ProjectRuntimeRequest::FinishCommit {
                project_id: request_project_id,
                live_revision,
                ..
            } if request_project_id == project_id => {
                runtime_finish_response(world, &request_id, live_revision)
            }
            ProjectRuntimeRequest::AbortCommit {
                project_id: request_project_id,
                ..
            } if request_project_id == project_id => {
                ProjectRuntimeResponse::Finished { request_id }
            }
            _ => ProjectRuntimeResponse::Inactive { request_id },
        };
        if let Err(error) = queue.publish_response(&project_root, &response) {
            bevy::log::warn!("Project runtime authority response failed: {error:?}");
        }
    }
}

fn runtime_revision_response(
    world: &World,
    request_id: &str,
    expected_revision: u64,
) -> ProjectRuntimeResponse {
    let Some(live) = world.get_non_send::<usd_bevy::LiveStage>() else {
        return ProjectRuntimeResponse::Failed {
            request_id: request_id.to_owned(),
            code: project_protocol::ProjectWriteErrorCode::Busy,
        };
    };
    if live.current_revision().0 != expected_revision {
        return ProjectRuntimeResponse::Failed {
            request_id: request_id.to_owned(),
            code: project_protocol::ProjectWriteErrorCode::ConcurrentChange,
        };
    }
    ProjectRuntimeResponse::Validated {
        request_id: request_id.to_owned(),
    }
}

fn runtime_finish_response(
    world: &World,
    request_id: &str,
    expected_revision: u64,
) -> ProjectRuntimeResponse {
    match runtime_revision_response(world, request_id, expected_revision) {
        ProjectRuntimeResponse::Validated { .. } => ProjectRuntimeResponse::Finished {
            request_id: request_id.to_owned(),
        },
        response => response,
    }
}

fn runtime_snapshot_response(
    world: &World,
    request_id: &str,
    active_scene_id: SceneId,
) -> ProjectRuntimeResponse {
    let Some(live) = world.get_non_send::<usd_bevy::LiveStage>() else {
        return ProjectRuntimeResponse::Failed {
            request_id: request_id.to_owned(),
            code: project_protocol::ProjectWriteErrorCode::Busy,
        };
    };
    let Ok(root_layer) = live.stage.root_layer().export_to_string() else {
        return ProjectRuntimeResponse::Failed {
            request_id: request_id.to_owned(),
            code: project_protocol::ProjectWriteErrorCode::ExportFailed,
        };
    };
    ProjectRuntimeResponse::Ready {
        request_id: request_id.to_owned(),
        lease_id: uuid::Uuid::new_v4().to_string(),
        scene_id: active_scene_id,
        live_revision: live.current_revision().0,
        root_layer: root_layer.into_bytes(),
    }
}

/// Apply canonical Project composition changes on the thread that owns the
/// actual LiveStage. The normal LiveStage drain/reconcile systems then observe
/// the resulting StageChangeBatch.
pub(super) fn consume_project_stage_mutations(world: &mut World) {
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
    let active_project_id: ProjectId = manifest.raw().project_id;
    let Some(active_scene_id) = active_scene_id_for_stage(&stage_path, &project_root) else {
        return;
    };
    let queue = world.resource::<ProjectStageMutationRuntime>().0.clone();
    let Some(mut live) = world.get_non_send_mut::<usd_bevy::LiveStage>() else {
        return;
    };
    if let Err(error) = queue.apply_for_active_scene(
        &mut *live,
        &project_root,
        active_project_id,
        Some(active_scene_id),
    ) {
        bevy::log::warn!("Project stage mutation handoff is waiting for retry: {error:?}");
    }
}

fn project_root_for_stage(stage_path: &Path) -> Option<PathBuf> {
    stage_path
        .ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == ".usdhub"))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

fn active_scene_id_for_stage(stage_path: &Path, project_root: &Path) -> Option<SceneId> {
    let scenes_directory = project_root.join(".usdhub").join("scenes");
    let relative = stage_path.strip_prefix(&scenes_directory).ok()?;
    if relative.components().count() != 1 || relative.extension().is_none_or(|ext| ext != "usda") {
        return None;
    }
    let scene_id = SceneId::parse(relative.file_stem()?.to_str()?).ok()?;
    (crate::project::scene::authoring::scene_path(project_root, scene_id) == stage_path)
        .then_some(scene_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_root_is_derived_only_for_a_project_scene_path() {
        let path = Path::new("/tmp/Project/.usdhub/scenes/scene.usda");
        assert_eq!(
            project_root_for_stage(path),
            Some(PathBuf::from("/tmp/Project"))
        );
        assert!(project_root_for_stage(Path::new("/tmp/scene.usda")).is_none());
    }

    #[test]
    fn active_scene_identity_requires_the_canonical_scene_locator() {
        let project_root = Path::new("/tmp/Project");
        let scene_id = SceneId::new_v4();
        let canonical = crate::project::scene::authoring::scene_path(project_root, scene_id);
        assert_eq!(
            active_scene_id_for_stage(&canonical, project_root),
            Some(scene_id)
        );
        assert!(
            active_scene_id_for_stage(
                &project_root.join(".usdhub/scenes/scene.usda"),
                project_root,
            )
            .is_none()
        );
        assert!(
            active_scene_id_for_stage(&project_root.join(".usdhub/project.usda"), project_root,)
                .is_none()
        );
    }
}
