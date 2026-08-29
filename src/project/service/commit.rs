//! Git-authoritative Project and Scene commit transactions.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use project_protocol::{
    ProjectCommitRequest, ProjectCommitResponse, ProjectCommitTarget, ProjectReadError,
    ProjectWriteError, ProjectWriteErrorCode,
};
use usd_git::{CommitRequest, GitRepository, Repository};
use usd_project::{SceneId, SceneMemberTarget};

use super::ProjectApplicationService;

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
    let mut repository = Repository::open(&project_root).map_err(|_| repository_error())?;
    if !repository
        .working_tree_status()
        .map_err(|_| repository_error())?
        .dirty
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
    }
    match request.target {
        ProjectCommitTarget::Project => {
            synchronize_tree(&project_root, staging.path(), Path::new(""))?;
        }
        ProjectCommitTarget::Scene(scene_id) => {
            synchronize_scene_scope(&project_root, staging.path(), &manifest, scene_id)?;
        }
    }
    let revision = repository
        .create_commit(CommitRequest::new(request.message.trim(), staging.path()))
        .map_err(|_| commit_error())?;
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
    let mut members = HashMap::new();
    for scene in manifest.scenes() {
        let path = crate::project::scene::authoring::scene_path(project_root, scene.id);
        let value = crate::project::scene::authoring::read_scene_members(&path, scene.id)
            .map_err(|_| commit_error())?;
        members.insert(scene.id, value);
    }
    let mut scenes = HashSet::from([root_scene]);
    let mut models = HashSet::new();
    let mut pending = vec![root_scene];
    while let Some(scene_id) = pending.pop() {
        for member in members.get(&scene_id).into_iter().flatten() {
            match member.target {
                SceneMemberTarget::Scene(child) if scenes.insert(child) => pending.push(child),
                SceneMemberTarget::Model(model_id) => {
                    models.insert(model_id);
                }
                SceneMemberTarget::Scene(_) => {}
            }
        }
    }
    for (parent, scene_members) in &members {
        if scene_members.iter().any(|member| {
            matches!(member.target, SceneMemberTarget::Scene(child) if scenes.contains(&child))
        }) {
            scenes.insert(*parent);
        }
    }
    synchronize_path(project_root, staging, Path::new(MANIFEST_RELATIVE_PATH))?;
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
