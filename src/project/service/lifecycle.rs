use std::{fs, path::Path};

use project_protocol::{
    ProjectInspection, ProjectInspectionClassification, ProjectWriteError, ProjectWriteErrorCode,
};
use usd_project::{ProjectManifestV1, ProjectRoot, ProjectSummary};

use super::ProjectApplicationService;
use crate::project::{
    catalog::manifest_store::ManifestStore,
    storage::{IgnoreChange, ProjectStorageLayout, install_managed_ignore},
};

impl ProjectApplicationService {
    pub fn inspect_project(
        &self,
        project_root: &Path,
    ) -> Result<ProjectInspection, ProjectWriteError> {
        super::inspection::inspect_project(project_root)
    }

    pub fn create_project(
        &mut self,
        parent: &Path,
        name: &str,
    ) -> Result<ProjectSummary, ProjectWriteError> {
        create_project(self, parent, name)
    }

    pub fn import_project(
        &mut self,
        project_root: &Path,
        expected: &ProjectInspection,
    ) -> Result<ProjectSummary, ProjectWriteError> {
        import_project(self, project_root, expected)
    }

    pub fn create_scene(
        &mut self,
        project_id: usd_project::ProjectId,
        target: project_protocol::ProjectWriteTarget,
        name: &str,
    ) -> Result<project_protocol::ProjectSceneWriteResponse, ProjectWriteError> {
        super::scene::create_scene(self, project_id, target, name)
    }

    pub fn adopt_scene(
        &mut self,
        project_id: usd_project::ProjectId,
        target: project_protocol::ProjectWriteTarget,
        source: &Path,
        inspection: &usd_project::CompositionInspection,
        operation_id: String,
        generation: u64,
    ) -> Result<project_protocol::ProjectSceneAdoptionResponse, ProjectWriteError> {
        super::scene_adoption::adopt_scene(
            self,
            project_id,
            target,
            source,
            inspection,
            operation_id,
            generation,
        )
    }

    pub fn publish_model(
        &mut self,
        preparation: &super::ProjectModelPreparationQueue,
        project_id: usd_project::ProjectId,
        target: project_protocol::ProjectWriteTarget,
        source: &std::path::Path,
        operation_id: String,
        generation: u64,
    ) -> Result<project_protocol::ProjectModelWriteResponse, ProjectWriteError> {
        super::model::publish_model(
            self,
            preparation,
            project_id,
            target,
            source,
            operation_id,
            generation,
        )
    }
}

/// Create a Project under an already-selected parent directory.
pub(super) fn create_project(
    service: &mut ProjectApplicationService,
    parent: &Path,
    name: &str,
) -> Result<ProjectSummary, ProjectWriteError> {
    validate_project_name(name)?;
    if !parent.is_dir() {
        return Err(ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::SelectionUnavailable,
        });
    }

    let project_root = parent.join(name);
    if project_root.exists() {
        return Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::ProjectAlreadyExists,
        });
    }

    let mut journal = CreateProjectJournal::default();
    fs::create_dir(&project_root).map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::FilesystemFailure,
    })?;
    journal.project_dir_created = true;

    let result = (|| {
        usd_git::Repository::init(&project_root).map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::RepositoryUnavailable,
        })?;
        let _ignore =
            install_managed_ignore(&project_root).map_err(|_| ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::FilesystemFailure,
            })?;

        let layout = ProjectStorageLayout::new(&project_root);
        layout
            .ensure_local_state_roots()
            .map_err(|_| ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::FilesystemFailure,
            })?;
        let project_id = usd_project::ProjectId::new_v4();
        let manifest =
            ProjectManifestV1::new(project_id, name, ProjectRoot::Empty, Vec::new(), Vec::new());
        ManifestStore::write_manifest_atomic(&project_root, &manifest).map_err(|_| {
            ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::ManifestUnavailable,
            }
        })?;

        let reopened =
            usd_git::Repository::open(&project_root).map_err(|_| ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::RepositoryUnavailable,
            })?;
        use usd_git::GitRepository;
        if reopened
            .head()
            .map_err(|_| ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::RepositoryUnavailable,
            })?
            .is_some()
        {
            return Err(ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::RepositoryUnavailable,
            });
        }
        let validated = ManifestStore::read_validated(&project_root).map_err(|_| {
            ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::ManifestUnavailable,
            }
        })?;
        let summary = super::inspection::project_summary(validated.raw(), &project_root)?;

        service
            .registry
            .register(project_id, &project_root, None)
            .map_err(|_| ProjectWriteError::RegistrationFailed {
                project_created: true,
            })?;
        journal.registry_entry_added = true;
        Ok(summary)
    })();

    match result {
        Ok(summary) => Ok(summary),
        Err(error) => {
            if journal.registry_entry_added
                || matches!(error, ProjectWriteError::RegistrationFailed { .. })
            {
                return Err(error);
            }
            journal.rollback(&project_root);
            Err(error)
        }
    }
}

pub(super) fn import_project(
    service: &mut ProjectApplicationService,
    project_root: &Path,
    expected: &ProjectInspection,
) -> Result<ProjectSummary, ProjectWriteError> {
    let current = super::inspection::inspect_project(project_root)?;
    if current != *expected {
        return Err(ProjectWriteError::ConcurrentChange);
    }

    match current.classification {
        ProjectInspectionClassification::NativeUsdHub => {
            let manifest = ManifestStore::read_validated(project_root).map_err(|_| {
                ProjectWriteError::Failed {
                    code: ProjectWriteErrorCode::ManifestUnavailable,
                }
            })?;
            let layout = ProjectStorageLayout::new(project_root);
            let ignore = install_managed_ignore(project_root).map_err(|error| {
                let message = error.to_string();
                if message.contains("broad") {
                    ProjectWriteError::Invalid {
                        code: ProjectWriteErrorCode::IgnoreConflict,
                    }
                } else {
                    ProjectWriteError::Failed {
                        code: ProjectWriteErrorCode::FilesystemFailure,
                    }
                }
            })?;
            if let Err(_) = layout.ensure_local_state_roots() {
                restore_ignore(project_root, ignore);
                return Err(ProjectWriteError::Failed {
                    code: ProjectWriteErrorCode::FilesystemFailure,
                });
            }
            let summary = super::inspection::project_summary(manifest.raw(), project_root)?;
            service
                .registry
                .register(manifest.raw().project_id, project_root, None)
                .map_err(|_| ProjectWriteError::RegistrationFailed {
                    project_created: true,
                })?;
            Ok(summary)
        }
        ProjectInspectionClassification::AdoptableGit => {
            adopt_git_project(service, project_root, &current)
        }
        ProjectInspectionClassification::Incompatible => Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::IncompatibleRepository,
        }),
    }
}

fn adopt_git_project(
    service: &mut ProjectApplicationService,
    project_root: &Path,
    inspection: &ProjectInspection,
) -> Result<ProjectSummary, ProjectWriteError> {
    let layout = ProjectStorageLayout::new(project_root);
    let had_manifest = layout.manifest_path().exists();
    let had_cache = layout.cache_dir().exists();
    let had_recovery = layout.recovery_dir().exists();
    let ignore = install_managed_ignore(project_root).map_err(|error| {
        if error.to_string().contains("broad") {
            ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::IgnoreConflict,
            }
        } else {
            ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::FilesystemFailure,
            }
        }
    })?;
    let project_id = usd_project::ProjectId::new_v4();
    let manifest = ProjectManifestV1::new(
        project_id,
        &inspection.display_name,
        ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    let result = (|| {
        ManifestStore::write_manifest_atomic(project_root, &manifest).map_err(|_| {
            ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::ManifestUnavailable,
            }
        })?;
        layout
            .ensure_local_state_roots()
            .map_err(|_| ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::FilesystemFailure,
            })?;
        let validated =
            ManifestStore::read_validated(project_root).map_err(|_| ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::ManifestUnavailable,
            })?;
        let summary = super::inspection::project_summary(validated.raw(), project_root)?;
        service
            .registry
            .register(project_id, project_root, None)
            .map_err(|_| ProjectWriteError::RegistrationFailed {
                project_created: true,
            })?;
        Ok(summary)
    })();

    if result.is_err() && !matches!(&result, Err(ProjectWriteError::RegistrationFailed { .. })) {
        if !had_manifest {
            let _ = fs::remove_file(layout.manifest_path());
        }
        if !had_cache {
            let _ = fs::remove_dir(layout.cache_dir());
        }
        if !had_recovery {
            let _ = fs::remove_dir(layout.recovery_dir());
        }
        if ignore.changed {
            restore_ignore(project_root, ignore);
        }
    }
    result
}

fn validate_project_name(name: &str) -> Result<(), ProjectWriteError> {
    if name.trim().is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::InvalidProjectName,
        });
    }
    Ok(())
}

fn restore_ignore(project_root: &Path, change: IgnoreChange) {
    let _ = crate::project::storage::restore_gitignore(project_root, &change);
}

#[derive(Default)]
struct CreateProjectJournal {
    project_dir_created: bool,
    registry_entry_added: bool,
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "lifecycle_m15_tests.rs"]
mod m15_tests;

impl CreateProjectJournal {
    fn rollback(&self, project_root: &Path) {
        if self.project_dir_created && !self.registry_entry_added {
            let _ = fs::remove_dir_all(project_root);
        }
    }
}
