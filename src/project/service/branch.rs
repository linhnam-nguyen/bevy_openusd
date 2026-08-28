use std::path::Path;

use project_protocol::{ProjectBranchSwitchResponse, ProjectWriteError, ProjectWriteErrorCode};
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
        ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::InvalidBranchName,
        }
    })?;
    let entry = service
        .registry
        .get(project_id)
        .ok_or(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::SelectionUnavailable,
        })?;
    let project_root = entry.repository_locator().to_owned();
    let publisher = service.publication_coordinator.publisher(project_id);
    let _guard = publisher.lock().map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::Busy,
    })?;

    let mut repository =
        usd_git::Repository::open(&project_root).map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::RepositoryUnavailable,
        })?;
    let status = usd_git::GitRepository::working_tree_status(&repository).map_err(|_| {
        ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::RepositoryUnavailable,
        }
    })?;
    if status.dirty {
        return Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::DirtyWorkingTree,
        });
    }

    usd_git::GitRepository::switch_branch(&mut repository, &branch).map_err(|error| {
        let code = match error {
            usd_git::Error::BranchNotFound(_) => ProjectWriteErrorCode::BranchNotFound,
            usd_git::Error::InvalidBranchName(_) => ProjectWriteErrorCode::InvalidBranchName,
            usd_git::Error::DirtyWorkingTree => ProjectWriteErrorCode::DirtyWorkingTree,
            _ => ProjectWriteErrorCode::BranchSwitchFailed,
        };
        ProjectWriteError::Failed { code }
    })?;

    let manifest = match validated_branch_manifest(project_id, &project_root) {
        Ok(manifest) => manifest,
        Err(_) => return Err(branch_project_invalid(project_id, &project_root)),
    };
    let (nodes, counts) = match super::project_tree(&project_root, &manifest) {
        Ok(projection) => projection,
        Err(_) => return Err(branch_project_invalid(project_id, &project_root)),
    };
    let repository_summary = match super::repository_summary(project_id, &project_root) {
        Ok(repository) => repository,
        Err(_) => return Err(branch_project_invalid(project_id, &project_root)),
    };
    let mut project = match super::inspection::project_summary(manifest.raw(), &project_root) {
        Ok(project) => project,
        Err(_) => return Err(branch_project_invalid(project_id, &project_root)),
    };
    project.repository = repository_summary.clone();
    project.counts = counts;
    Ok(ProjectBranchSwitchResponse {
        project,
        repository: repository_summary,
        nodes,
        counts,
    })
}

fn branch_project_invalid(project_id: ProjectId, project_root: &Path) -> ProjectWriteError {
    match super::repository_summary(project_id, project_root) {
        Ok(repository) => ProjectWriteError::BranchProjectInvalid {
            repository: Box::new(repository),
        },
        Err(_) => ProjectWriteError::BranchProjectTruthUnavailable,
    }
}

fn validated_branch_manifest(
    project_id: ProjectId,
    project_root: &Path,
) -> Result<usd_project::ValidatedProjectManifest, ProjectWriteError> {
    let manifest =
        ManifestStore::read_validated(project_root).map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::BranchProjectInvalid,
        })?;
    if manifest.raw().project_id != project_id {
        return Err(ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::BranchProjectInvalid,
        });
    }
    Ok(manifest)
}
