use std::{fs, path::Path};

use anyhow::Context;
use project_protocol::{ProjectBranchSwitchResponse, ProjectWriteError, ProjectWriteErrorCode};
use usd_git::GitRepository;
use usd_project::ProjectId;

use super::ProjectApplicationService;
use crate::project::catalog::manifest_store::ManifestStore;

impl ProjectApplicationService {
    /// Switch one registered Project to an existing local branch and return
    /// the complete authoritative projection for the new branch.
    pub fn switch_branch(
        &mut self,
        project_id: ProjectId,
        branch_name: &str,
    ) -> Result<ProjectBranchSwitchResponse, ProjectWriteError> {
        switch_branch(self, project_id, branch_name)
    }
}
fn switch_branch(
    service: &mut ProjectApplicationService,
    project_id: ProjectId,
    branch_name: &str,
) -> Result<ProjectBranchSwitchResponse, ProjectWriteError> {
    let branch = usd_git::BranchName::new(branch_name.to_owned()).map_err(|_| {
        ProjectWriteError::Invalid { code: ProjectWriteErrorCode::InvalidBranchName }
    })?;
    let entry = service.registry.get(project_id).ok_or(ProjectWriteError::Invalid {
        code: ProjectWriteErrorCode::SelectionUnavailable,
    })?;
    let project_root = entry.repository_locator().to_owned();
    let publisher = service.publication_coordinator.publisher(project_id);
    let _guard = publisher.lock().map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::Busy,
    })?;

    let mut repository = usd_git::Repository::open(&project_root).map_err(|_| {
        ProjectWriteError::Failed { code: ProjectWriteErrorCode::RepositoryUnavailable }
    })?;
    if repository.working_tree_status().map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::RepositoryUnavailable,
    })?.dirty {
        return Err(ProjectWriteError::Invalid { code: ProjectWriteErrorCode::DirtyWorkingTree });
    }
    let previous_manifest = validated_branch_manifest(project_id, &project_root).ok();
    let previous_revision = repository.head().map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::RepositoryUnavailable,
    })?;

    let outcome = repository.switch_branch(&branch).map_err(|error| {
        let code = match error {
            usd_git::Error::BranchNotFound(_) => ProjectWriteErrorCode::BranchNotFound,
            usd_git::Error::InvalidBranchName(_) => ProjectWriteErrorCode::InvalidBranchName,
            usd_git::Error::DirtyWorkingTree => ProjectWriteErrorCode::DirtyWorkingTree,
            _ => ProjectWriteErrorCode::BranchSwitchFailed,
        };
        ProjectWriteError::Failed { code }
    })?;
    let switched = matches!(outcome, usd_git::BranchSwitchOutcome::Switched { .. });

    let result = post_switch_projection(
        service,
        project_id,
        &project_root,
        &repository,
        previous_manifest.as_ref(),
        previous_revision.as_ref().map(usd_git::Revision::id),
    );
    if result.is_err() && switched {
        invalidate_previous_branch_cache(service, &project_root, previous_manifest.as_ref());
    }
    result
}

fn post_switch_projection(
    service: &mut ProjectApplicationService,
    project_id: ProjectId,
    project_root: &Path,
    repository: &usd_git::Repository,
    previous_manifest: Option<&usd_project::ValidatedProjectManifest>,
    previous_revision: Option<&usd_git::RevisionId>,
) -> Result<ProjectBranchSwitchResponse, ProjectWriteError> {
    let current_revision = repository.head().map_err(|_| branch_project_invalid(project_id, project_root))?;
    let changed_paths = match (previous_revision, current_revision.as_ref()) {
        (Some(previous), Some(current)) => repository
            .changed_paths_between(previous, current.id())
            .map_err(|_| branch_project_invalid(project_id, project_root))?,
        _ => Vec::new(),
    };
    let manifest = validated_branch_manifest(project_id, project_root)?;
    crate::project::scene::root::ensure_protected_root_scene_atomic(project_root, manifest.raw())
        .map_err(|_| branch_project_invalid(project_id, project_root))?;
    let manifest = validated_branch_manifest(project_id, project_root)?;

    let previous_scene_ids = previous_branch_scene_ids(project_root, previous_manifest)
        .map_err(|error| {
            log::error!(
                "previous-branch Scene cache discovery failed for {}: {error:#}",
                project_root.display()
            );
            branch_project_invalid(project_id, project_root)
        })?;
    for scene_id in previous_scene_ids
        .into_iter()
        .filter(|scene_id| manifest.scene(*scene_id).is_none())
    {
        let removed = service.cache_warm.remove_target_descriptors(
            project_root,
            &crate::project::cache::ProjectCacheTarget::Scene { id: scene_id.to_string() },
        );
        if !removed {
            log::error!(
                "previous-branch Scene cache removal failed for {} ({scene_id})",
                project_root.display()
            );
            return Err(branch_project_invalid(project_id, project_root));
        }
    }
    let mut cache_targets = vec![crate::project::cache::ProjectCacheTarget::ProjectRoot];
    if let Some(previous_manifest) = previous_manifest {
        cache_targets.extend(crate::project::cache_warmer::cache_targets_for_changed_paths(
            project_root, previous_manifest, &changed_paths,
        ));
    }
    cache_targets.extend(crate::project::cache_warmer::cache_targets_for_changed_paths(
        project_root, &manifest, &changed_paths,
    ));
    cache_targets.retain(|target| match target {
        crate::project::cache::ProjectCacheTarget::Scene { id } => usd_project::SceneId::parse(id)
            .ok().is_some_and(|scene_id| manifest.scene(scene_id).is_some()),
        crate::project::cache::ProjectCacheTarget::Model { id } => usd_project::ModelId::parse(id)
            .ok().is_some_and(|model_id| manifest.model(model_id).is_some()),
        crate::project::cache::ProjectCacheTarget::ProjectRoot => true,
    });
    cache_targets.sort_by_key(crate::project::cache::ProjectCacheTarget::key);
    cache_targets.dedup_by_key(|target| target.key());
    match service
        .cache_warm
        .enqueue_targets_for_mutation(project_root, cache_targets)
    {
        Ok(true) => {}
        Ok(false) => {
            log::warn!(
                "current-branch Scene cache warm admission was unavailable for {}",
                project_root.display()
            );
        }
        Err(error) => {
            log::error!(
                "current-branch Scene cache boundary failed for {}: {error:#}",
                project_root.display()
            );
            return Err(branch_project_invalid(project_id, project_root));
        }
    }

    let (nodes, counts) = super::project_tree(project_root, &manifest)
        .map_err(|_| branch_project_invalid(project_id, project_root))?;
    let repository_summary = super::repository_summary(project_id, project_root)
        .map_err(|_| branch_project_invalid(project_id, project_root))?;
    let mut project = super::inspection::project_summary(manifest.raw(), project_root)
        .map_err(|_| branch_project_invalid(project_id, project_root))?;
    project.repository = repository_summary.clone();
    project.counts = counts;
    Ok(ProjectBranchSwitchResponse { project, repository: repository_summary, nodes, counts })
}

fn previous_branch_scene_ids(
    project_root: &Path,
    previous_manifest: Option<&usd_project::ValidatedProjectManifest>,
) -> anyhow::Result<std::collections::BTreeSet<usd_project::SceneId>> {
    let mut scene_ids = std::collections::BTreeSet::new();
    if let Some(previous_manifest) = previous_manifest {
        scene_ids.extend(previous_manifest.scenes().iter().map(|scene| scene.id));
    }
    let scene_cache_root = crate::project::storage::ProjectStorageLayout::new(project_root)
        .cache_dir().join("scenes");
    let entries = match fs::read_dir(&scene_cache_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(scene_ids),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("read previous-branch Scene cache directory {}", scene_cache_root.display())
            });
        }
    };
    for entry in entries {
        let entry = entry.with_context(|| {
            format!("read previous-branch Scene cache entry {}", scene_cache_root.display())
        })?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str()
            && let Ok(scene_id) = usd_project::SceneId::parse(name)
        {
            scene_ids.insert(scene_id);
        }
    }
    Ok(scene_ids)
}

fn invalidate_previous_branch_cache(
    service: &ProjectApplicationService,
    project_root: &Path,
    previous_manifest: Option<&usd_project::ValidatedProjectManifest>,
) {
    let scene_ids = match previous_branch_scene_ids(project_root, previous_manifest) {
        Ok(scene_ids) => scene_ids,
        Err(error) => {
            log::error!(
                "previous-branch Scene cache discovery failed during error recovery for {}: {error:#}",
                project_root.display()
            );
            if let Err(cleanup_error) = crate::project::cache::SceneCacheStore::new(project_root)
                .remove_all_derived_cache()
            {
                log::error!(
                    "conservative previous-branch cache removal also failed for {}: {cleanup_error:#}",
                    project_root.display()
                );
            }
            return;
        }
    };
    for scene_id in scene_ids {
        if !service.cache_warm.remove_target_descriptors(
            project_root,
            &crate::project::cache::ProjectCacheTarget::Scene { id: scene_id.to_string() },
        ) {
            log::error!(
                "previous-branch Scene cache removal failed during error recovery for {} ({scene_id})",
                project_root.display()
            );
        }
    }
    if !service.cache_warm.remove_target_descriptors(
        project_root,
        &crate::project::cache::ProjectCacheTarget::ProjectRoot,
    ) {
        log::error!(
            "Project-root cache removal failed during previous-branch error recovery for {}",
            project_root.display()
        );
    }
    let index = crate::project::storage::ProjectStorageLayout::new(project_root).project_cache_index_path();
    match fs::remove_file(index) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("failed to remove stale Project cache lookup after branch error: {error}"),
    }
}

fn branch_project_invalid(project_id: ProjectId, project_root: &Path) -> ProjectWriteError {
    match super::repository_summary(project_id, project_root) {
        Ok(repository) => ProjectWriteError::BranchProjectInvalid { repository: Box::new(repository) },
        Err(_) => ProjectWriteError::BranchProjectTruthUnavailable,
    }
}

fn validated_branch_manifest(
    project_id: ProjectId,
    project_root: &Path,
) -> Result<usd_project::ValidatedProjectManifest, ProjectWriteError> {
    let manifest = ManifestStore::read_validated(project_root)
        .map_err(|_| branch_project_invalid(project_id, project_root))?;
    if manifest.raw().project_id != project_id {
        return Err(branch_project_invalid(project_id, project_root));
    }
    Ok(manifest)
}
