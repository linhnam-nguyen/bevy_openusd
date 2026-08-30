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

#[path = "project_runtime_authority.rs"]
mod project_runtime_authority;

/// The render-server process owns this queue resource. Project mutation
/// records are read from the active Project's private cache outbox.
#[derive(Resource, Clone, Default)]
pub(super) struct ProjectStageMutationRuntime(pub(super) ProjectStageMutationQueue);

#[derive(Clone, Debug)]
struct ActiveProjectRuntimeLease {
    project_id: ProjectId,
    lease_id: String,
    session_id: u64,
    live_revision: u64,
}

#[derive(Resource, Clone)]
pub(super) struct ProjectRuntimeAuthorityRuntime {
    pub(super) queue: ProjectRuntimeAuthorityQueue,
    active_lease: Option<ActiveProjectRuntimeLease>,
}

impl Default for ProjectRuntimeAuthorityRuntime {
    fn default() -> Self {
        Self {
            queue: ProjectRuntimeAuthorityQueue::default(),
            active_lease: None,
        }
    }
}

pub(super) fn install(app: &mut App) {
    app.insert_resource(ProjectStageMutationRuntime::default())
        .insert_resource(ProjectRuntimeAuthorityRuntime::default())
        .add_systems(
            Update,
            (
                project_runtime_authority::consume_project_runtime_authority,
                consume_project_stage_mutations,
            )
                .in_set(usd_bevy::LiveStageSet::Project),
        );
}

fn runtime_revision_response(
    world: &World,
    request_id: &str,
    project_id: ProjectId,
    lease_id: &str,
    expected_revision: u64,
) -> ProjectRuntimeResponse {
    let Some(active_lease) = world
        .get_resource::<ProjectRuntimeAuthorityRuntime>()
        .and_then(|runtime| runtime.active_lease.as_ref())
    else {
        return runtime_failed(request_id, project_protocol::ProjectWriteErrorCode::Busy);
    };
    if active_lease.project_id != project_id || active_lease.lease_id != lease_id {
        return runtime_failed(request_id, project_protocol::ProjectWriteErrorCode::Busy);
    }
    if active_lease.live_revision != expected_revision {
        return runtime_failed(
            request_id,
            project_protocol::ProjectWriteErrorCode::ConcurrentChange,
        );
    }
    let Some(live) = world.get_non_send::<usd_bevy::LiveStage>() else {
        return runtime_failed(request_id, project_protocol::ProjectWriteErrorCode::Busy);
    };
    if live.session_id() != active_lease.session_id {
        return runtime_failed(request_id, project_protocol::ProjectWriteErrorCode::Busy);
    }
    if live.authoring_generation().0 != expected_revision {
        return runtime_failed(
            request_id,
            project_protocol::ProjectWriteErrorCode::ConcurrentChange,
        );
    }
    ProjectRuntimeResponse::Validated {
        request_id: request_id.to_owned(),
    }
}

fn runtime_finish_response(
    world: &mut World,
    request_id: &str,
    project_id: ProjectId,
    lease_id: &str,
    expected_revision: u64,
) -> ProjectRuntimeResponse {
    match runtime_revision_response(world, request_id, project_id, lease_id, expected_revision) {
        ProjectRuntimeResponse::Validated { .. } => {
            if let Some(live) = world.get_non_send::<usd_bevy::LiveStage>() {
                live.unfreeze_authoring();
            }
            world
                .resource_mut::<ProjectRuntimeAuthorityRuntime>()
                .active_lease = None;
            ProjectRuntimeResponse::Finished {
                request_id: request_id.to_owned(),
            }
        }
        response => response,
    }
}

fn runtime_snapshot_response(
    world: &mut World,
    request_id: &str,
    project_id: ProjectId,
    active_scene_id: SceneId,
) -> ProjectRuntimeResponse {
    if world
        .resource::<ProjectRuntimeAuthorityRuntime>()
        .active_lease
        .is_some()
    {
        return runtime_failed(request_id, project_protocol::ProjectWriteErrorCode::Busy);
    }
    let (root_layer, lease_id, session_id, live_revision) = {
        let Some(live) = world.get_non_send::<usd_bevy::LiveStage>() else {
            return runtime_failed(request_id, project_protocol::ProjectWriteErrorCode::Busy);
        };
        if !live.try_freeze_authoring() {
            return runtime_failed(request_id, project_protocol::ProjectWriteErrorCode::Busy);
        }
        let Ok(root_layer) = live.stage.root_layer().export_to_string() else {
            live.unfreeze_authoring();
            return runtime_failed(
                request_id,
                project_protocol::ProjectWriteErrorCode::ExportFailed,
            );
        };
        (
            root_layer,
            uuid::Uuid::new_v4().to_string(),
            live.session_id(),
            live.authoring_generation().0,
        )
    };
    world
        .resource_mut::<ProjectRuntimeAuthorityRuntime>()
        .active_lease = Some(ActiveProjectRuntimeLease {
        project_id,
        lease_id: lease_id.clone(),
        session_id,
        live_revision,
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

fn runtime_abort_response(
    world: &mut World,
    request_id: &str,
    project_id: ProjectId,
    lease_id: &str,
) -> ProjectRuntimeResponse {
    let matches = world
        .resource::<ProjectRuntimeAuthorityRuntime>()
        .active_lease
        .as_ref()
        .is_some_and(|active| active.project_id == project_id && active.lease_id == lease_id);
    if !matches {
        return runtime_failed(request_id, project_protocol::ProjectWriteErrorCode::Busy);
    }
    clear_runtime_lease(world, project_id, lease_id);
    ProjectRuntimeResponse::Finished {
        request_id: request_id.to_owned(),
    }
}

fn clear_runtime_lease(world: &mut World, project_id: ProjectId, lease_id: &str) {
    let matches = world
        .resource::<ProjectRuntimeAuthorityRuntime>()
        .active_lease
        .as_ref()
        .is_some_and(|active| active.project_id == project_id && active.lease_id == lease_id);
    if !matches {
        return;
    }
    if let Some(live) = world.get_non_send::<usd_bevy::LiveStage>() {
        live.unfreeze_authoring();
    }
    world
        .resource_mut::<ProjectRuntimeAuthorityRuntime>()
        .active_lease = None;
}

fn runtime_failed(
    request_id: &str,
    code: project_protocol::ProjectWriteErrorCode,
) -> ProjectRuntimeResponse {
    ProjectRuntimeResponse::Failed {
        request_id: request_id.to_owned(),
        code,
    }
}

fn runtime_scene_is_allowed(
    project_root: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    active_scene_id: SceneId,
    target: &ProjectCommitTarget,
) -> bool {
    match target {
        ProjectCommitTarget::Project => manifest.scene(active_scene_id).is_some(),
        ProjectCommitTarget::Scene(scene_id) => {
            crate::project::service::scene_closure::scene_commit_closure(
                project_root,
                manifest.raw(),
                *scene_id,
            )
            .is_ok_and(|(scenes, _)| scenes.contains(&active_scene_id))
        }
    }
}

fn runtime_scene_is_in_export_closure(
    project_root: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    root_scene: SceneId,
    active_scene_id: SceneId,
) -> bool {
    crate::project::service::scene_closure::scene_dependency_closure(
        project_root,
        manifest.raw(),
        root_scene,
    )
    .is_ok_and(|(scenes, _)| scenes.contains(&active_scene_id))
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
    if live.is_authoring_frozen() {
        return;
    }
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
