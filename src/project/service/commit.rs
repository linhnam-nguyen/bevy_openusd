//! Git-authoritative Project and Scene commit transactions.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use project_protocol::{
    ProjectCommitRequest, ProjectCommitResponse, ProjectCommitTarget, ProjectReadError,
    ProjectWriteError, ProjectWriteErrorCode,
};
use usd_git::{CommitRequest, GitRepository, Repository};
use usd_project::{ProjectManifestV1, SceneId};

use super::ProjectApplicationService;
use super::commit_runtime::{
    RuntimeLeaseGuard, overlay_runtime_snapshot, persist_semantic_snapshot,
};

const MANIFEST_RELATIVE_PATH: &str = ".usdhub/project.json";
const SCENES_RELATIVE_DIRECTORY: &str = ".usdhub/scenes";
const MODELS_RELATIVE_DIRECTORY: &str = ".usdhub/models";
const SCENE_SOURCES_RELATIVE_DIRECTORY: &str = ".usdhub/imports/scenes";

pub(super) fn commit(
    service: &mut ProjectApplicationService,
    request: ProjectCommitRequest,
) -> Result<ProjectCommitResponse, ProjectWriteError> {
    let project_id = request.project_id;
    let (entry, manifest) = service
        .validated_project(project_id)
        .map_err(project_error)?;
    let project_root = entry.repository_locator().to_owned();
    validate_target(&manifest, &request.target)?;
    if request.message.trim().is_empty() {
        return Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::CommitMessageInvalid,
        });
    }

    let publisher = service.publication_coordinator.publisher(project_id);
    let _guard = publisher.lock().map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::Busy,
    })?;
    let runtime_authority = service.publication_coordinator.runtime_authority_arc();
    let runtime_snapshot =
        runtime_authority.begin_commit(&project_root, project_id, &request.target)?;
    let mut runtime_lease = RuntimeLeaseGuard::new(
        runtime_authority.clone(),
        project_root.clone(),
        project_id,
        runtime_snapshot.as_ref(),
    );
    let mut repository = Repository::open(&project_root).map_err(|_| repository_error())?;
    if !repository
        .working_tree_status()
        .map_err(|_| repository_error())?
        .dirty
        && runtime_snapshot.is_none()
    {
        return Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::NothingToCommit,
        });
    }

    let staging = tempfile::tempdir().map_err(|_| commit_error())?;
    if let Some(head) = repository.head().map_err(|_| repository_error())? {
        repository
            .materialize_revision(head.id(), staging.path())
            .map_err(|_| commit_error())?;
    } else {
        // A newly created Project has no Git baseline yet. Seed the first
        // scoped commit from the complete canonical tree so its manifest and
        // protected root are available before applying the closure overlay.
        synchronize_tree(&project_root, staging.path(), Path::new(""))?;
    }
    match request.target {
        ProjectCommitTarget::Project => {
            synchronize_tree(&project_root, staging.path(), Path::new(""))?;
        }
        ProjectCommitTarget::Scene(scene_id) => {
            synchronize_scene_scope(&project_root, staging.path(), &manifest, scene_id)?;
        }
    }
    if let Some(snapshot) = runtime_snapshot.as_ref() {
        runtime_authority.validate_commit(
            &project_root,
            project_id,
            &snapshot.lease_id,
            snapshot.live_revision,
        )?;
        overlay_runtime_snapshot(
            &project_root,
            staging.path(),
            &manifest,
            &request.target,
            snapshot,
        )?;
    }
    let revision = repository
        .create_commit(CommitRequest::new(request.message.trim(), staging.path()))
        .map_err(|_| commit_error())?;
    if let Some(snapshot) = runtime_snapshot.as_ref() {
        if let Err(error) = persist_semantic_snapshot(
            &project_root,
            staging.path(),
            snapshot.scene_id,
            &revision.to_string(),
        ) {
            log::warn!("committed LiveStage semantic persistence was deferred: {error:#}");
        }
        match runtime_authority.finish_commit(
            &project_root,
            project_id,
            &snapshot.lease_id,
            &revision.to_string(),
            snapshot.live_revision,
        ) {
            Ok(()) => runtime_lease.clear(),
            Err(error) => {
                log::warn!("LiveStage commit finalization was deferred: {error}");
            }
        }
    }
    let committed_manifest = super::ManifestStore::read_validated(&project_root).map_err(|_| {
        ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::ManifestUnavailable,
        }
    })?;
    let (_nodes, counts) =
        super::project_tree(&project_root, &committed_manifest).map_err(|_| commit_error())?;
    let repository_summary =
        super::repository_summary(project_id, &project_root).map_err(|_| repository_error())?;
    let mut project = super::inspection::project_summary(committed_manifest.raw(), &project_root)
        .map_err(|_| commit_error())?;
    project.counts = counts;
    project.repository = repository_summary.clone();
    let _ = service.cache_warm.enqueue_project_targets(&project_root);
    Ok(ProjectCommitResponse {
        project,
        repository: repository_summary,
        revision: usd_project::RevisionSummary {
            id: revision.to_string(),
        },
    })
}

fn validate_target(
    manifest: &usd_project::ValidatedProjectManifest,
    target: &ProjectCommitTarget,
) -> Result<(), ProjectWriteError> {
    match target {
        ProjectCommitTarget::Project => Ok(()),
        ProjectCommitTarget::Scene(scene_id) => manifest
            .scene(*scene_id)
            .is_some()
            .then_some(())
            .ok_or(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::SceneNotFound,
            }),
    }
}

fn synchronize_scene_scope(
    project_root: &Path,
    staging: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    root_scene: SceneId,
) -> Result<(), ProjectWriteError> {
    let (scenes, models) =
        super::scene_closure::scene_commit_closure(project_root, manifest.raw(), root_scene)
            .map_err(|_| commit_error())?;

    synchronize_manifest_scope(staging, manifest, &scenes, &models)?;
    for scene_id in scenes {
        synchronize_path(
            project_root,
            staging,
            &PathBuf::from(SCENES_RELATIVE_DIRECTORY).join(format!("{scene_id}.usda")),
        )?;
        synchronize_path(
            project_root,
            staging,
            &PathBuf::from(SCENE_SOURCES_RELATIVE_DIRECTORY).join(scene_id.to_string()),
        )?;
    }
    for model_id in models {
        synchronize_path(
            project_root,
            staging,
            &PathBuf::from(MODELS_RELATIVE_DIRECTORY).join(model_id.to_string()),
        )?;
    }
    Ok(())
}

fn synchronize_manifest_scope(
    staging: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    scenes: &HashSet<SceneId>,
    models: &HashSet<usd_project::ModelId>,
) -> Result<(), ProjectWriteError> {
    let manifest_path = staging.join(MANIFEST_RELATIVE_PATH);
    let bytes = fs::read(&manifest_path).map_err(|_| commit_error())?;
    let mut staged: ProjectManifestV1 =
        serde_json::from_slice(&bytes).map_err(|_| commit_error())?;
    let current = manifest.raw();

    for scene in current
        .scenes
        .iter()
        .filter(|scene| scenes.contains(&scene.id))
    {
        if let Some(staged_scene) = staged.scenes.iter_mut().find(|entry| entry.id == scene.id) {
            *staged_scene = scene.clone();
        } else {
            staged.scenes.push(scene.clone());
        }
    }
    for model in current
        .models
        .iter()
        .filter(|model| models.contains(&model.id))
    {
        if let Some(staged_model) = staged.models.iter_mut().find(|entry| entry.id == model.id) {
            *staged_model = model.clone();
        } else {
            staged.models.push(model.clone());
        }
    }
    staged.validate().map_err(|_| commit_error())?;
    let encoded = serde_json::to_vec_pretty(&staged.canonicalized()).map_err(|_| commit_error())?;
    fs::write(&manifest_path, encoded).map_err(|_| commit_error())?;
    Ok(())
}

fn synchronize_tree(
    source_root: &Path,
    destination_root: &Path,
    relative: &Path,
) -> Result<(), ProjectWriteError> {
    synchronize_path(source_root, destination_root, relative)
}

fn synchronize_path(
    source_root: &Path,
    destination_root: &Path,
    relative: &Path,
) -> Result<(), ProjectWriteError> {
    if is_excluded(relative) {
        return Ok(());
    }
    let source = source_root.join(relative);
    let destination = destination_root.join(relative);
    if !source.exists() {
        if destination.exists() {
            remove_path(&destination)?;
        }
        return Ok(());
    }
    let metadata = fs::symlink_metadata(&source).map_err(|_| commit_error())?;
    if metadata.is_dir() {
        if destination.exists() && !destination.is_dir() {
            remove_path(&destination)?;
        }
        fs::create_dir_all(&destination).map_err(|_| commit_error())?;
        for entry in fs::read_dir(&source).map_err(|_| commit_error())? {
            let entry = entry.map_err(|_| commit_error())?;
            let child = relative.join(entry.file_name());
            synchronize_path(source_root, destination_root, &child)?;
        }
        for entry in fs::read_dir(&destination).map_err(|_| commit_error())? {
            let entry = entry.map_err(|_| commit_error())?;
            let child = relative.join(entry.file_name());
            if is_excluded(&child) || source_root.join(&child).exists() {
                continue;
            }
            remove_path(&entry.path())?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|_| commit_error())?;
        }
        if destination.exists() {
            remove_path(&destination)?;
        }
        fs::copy(source, destination).map_err(|_| commit_error())?;
    } else {
        return Err(commit_error());
    }
    Ok(())
}

fn is_excluded(relative: &Path) -> bool {
    let text = relative.to_string_lossy();
    text == ".git"
        || text == ".usdhub/cache"
        || text.starts_with(".usdhub/cache/")
        || text == ".usdhub/recovery"
        || text.starts_with(".usdhub/recovery/")
        || text == ".usdhub/links"
        || text.starts_with(".usdhub/links/")
        || text == ".usdhub/.transactions"
        || text.starts_with(".usdhub/.transactions/")
}

fn remove_path(path: &Path) -> Result<(), ProjectWriteError> {
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|_| commit_error())
    } else {
        fs::remove_file(path).map_err(|_| commit_error())
    }
}

fn project_error(error: ProjectReadError) -> ProjectWriteError {
    match error {
        ProjectReadError::NotFound { .. } => ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::ProjectNotFound,
        },
        _ => repository_error(),
    }
}

fn repository_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::RepositoryUnavailable,
    }
}

fn commit_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::CommitFailed,
    }
}
