//! Render-server owner for the Project-to-LiveStage handoff.

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use project_protocol::ProjectCommitTarget;
use usd_project::{ProjectId, SceneId};

use crate::{
    project::{
        catalog::manifest_store::ManifestStore,
        service::{
            ProjectRuntimeAuthorityQueue, ProjectRuntimeResponse, ProjectStageMutationQueue,
        },
    },
    viewport::session::StageHandle,
};

#[path = "project_runtime_authority.rs"]
mod project_runtime_authority;
#[path = "project_runtime_snapshot.rs"]
mod project_runtime_snapshot;
pub(super) use project_runtime_snapshot::runtime_snapshot_response;
#[cfg(test)]
pub(super) use project_runtime_snapshot::runtime_snapshot_response_with_claim_hook;

/// The render-server process owns this queue resource. Project runtime
/// authority requests are read from the shared workspace inbox.
#[derive(Resource, Clone, Default)]
pub(super) struct ProjectStageMutationRuntime(pub(super) ProjectStageMutationQueue);

#[derive(Clone, Debug)]
struct ActiveProjectRuntimeLease {
    project_id: ProjectId,
    lease_id: String,
    session_id: u64,
    live_revision: u64,
    expires_at_ms: u128,
}

/// Time allowed after the last owner heartbeat before the render owner
/// releases a stranded LiveStage lease. Healthy operations are renewable.
const RUNTIME_LEASE_GRACE_MS: u128 = 30_000;

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
            clear_runtime_lease(world, project_id, lease_id);
            ProjectRuntimeResponse::Finished {
                request_id: request_id.to_owned(),
            }
        }
        response => response,
    }
}

fn runtime_renew_response(
    world: &mut World,
    request_id: &str,
    project_id: ProjectId,
    lease_id: &str,
    expected_revision: u64,
) -> ProjectRuntimeResponse {
    match runtime_revision_response(world, request_id, project_id, lease_id, expected_revision) {
        ProjectRuntimeResponse::Validated { .. } => {
            let mut runtime = world.resource_mut::<ProjectRuntimeAuthorityRuntime>();
            let Some(active_lease) = runtime
                .active_lease
                .as_mut()
                .filter(|active| active.project_id == project_id && active.lease_id == lease_id)
            else {
                return runtime_failed(request_id, project_protocol::ProjectWriteErrorCode::Busy);
            };
            active_lease.expires_at_ms =
                crate::project::service::unix_time_ms().saturating_add(RUNTIME_LEASE_GRACE_MS);
            ProjectRuntimeResponse::Renewed {
                request_id: request_id.to_owned(),
            }
        }
        response => response,
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

pub(super) fn expire_runtime_lease(world: &mut World) {
    let expired = world
        .resource::<ProjectRuntimeAuthorityRuntime>()
        .active_lease
        .as_ref()
        .filter(|lease| lease.expires_at_ms <= crate::project::service::unix_time_ms())
        .map(|lease| (lease.project_id, lease.lease_id.clone()));
    if let Some((project_id, lease_id)) = expired {
        clear_runtime_lease(world, project_id, &lease_id);
    }
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
#[path = "project_stage_tests.rs"]
mod tests;
