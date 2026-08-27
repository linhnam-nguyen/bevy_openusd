use std::{fs, path::Path};

use project_protocol::{
    ProjectInspection, ProjectInspectionClassification, ProjectInspectionWarning,
    ProjectWriteError, ProjectWriteErrorCode,
};
use usd_project::{
    ProjectCapabilities, ProjectContentCounts, ProjectManifestV1, ProjectRoot, ProjectSummary,
};

use super::{ProjectApplicationService, repository_summary};
use crate::project::{
    catalog::manifest_store::ManifestStore,
    storage::{
        IgnoreChange, ProjectStorageLayout, has_broad_usdhub_ignore, install_managed_ignore,
        read_gitignore,
    },
};

impl ProjectApplicationService {
    pub fn inspect_project(&self, project_root: &Path) -> Result<ProjectInspection, ProjectWriteError> {
        inspect_project(project_root)
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
        let _ignore = install_managed_ignore(&project_root).map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::FilesystemFailure,
        })?;

        let layout = ProjectStorageLayout::new(&project_root);
        layout.ensure_local_state_roots().map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::FilesystemFailure,
        })?;
        let project_id = usd_project::ProjectId::new_v4();
        let manifest = ProjectManifestV1::new(
            project_id,
            name,
            ProjectRoot::Empty,
            Vec::new(),
            Vec::new(),
        );
        ManifestStore::write_manifest_atomic(&project_root, &manifest).map_err(|_| {
            ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::ManifestUnavailable,
            }
        })?;

        let reopened = usd_git::Repository::open(&project_root).map_err(|_| {
            ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::RepositoryUnavailable,
            }
        })?;
        use usd_git::GitRepository;
        if reopened.head().map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::RepositoryUnavailable,
        })?.is_some() {
            return Err(ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::RepositoryUnavailable,
            });
        }
        let validated = ManifestStore::read_validated(&project_root).map_err(|_| {
            ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::ManifestUnavailable,
            }
        })?;
        let summary = project_summary(validated.raw(), &project_root)?;

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
            if journal.registry_entry_added {
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
    let current = inspect_project(project_root)?;
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
            let summary = project_summary(manifest.raw(), project_root)?;
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
    let had_ignore = project_root.join(".gitignore").exists();
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
        layout.ensure_local_state_roots().map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::FilesystemFailure,
        })?;
        let validated = ManifestStore::read_validated(project_root).map_err(|_| {
            ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::ManifestUnavailable,
            }
        })?;
        let summary = project_summary(validated.raw(), project_root)?;
        service
            .registry
            .register(project_id, project_root, None)
            .map_err(|_| ProjectWriteError::RegistrationFailed {
                project_created: true,
            })?;
        Ok(summary)
    })();

    if result.is_err() {
        if !had_manifest {
            let _ = fs::remove_file(layout.manifest_path());
        }
        if !had_cache {
            let _ = fs::remove_dir(layout.cache_dir());
        }
        if !had_recovery {
            let _ = fs::remove_dir(layout.recovery_dir());
        }
        if !had_ignore {
            restore_ignore(project_root, ignore);
        }
    }
    result
}

fn inspect_project(project_root: &Path) -> Result<ProjectInspection, ProjectWriteError> {
    let repository = match usd_git::Repository::open(project_root) {
        Ok(repository) => repository,
        Err(_) => {
            return Ok(ProjectInspection {
                classification: ProjectInspectionClassification::Incompatible,
                display_name: project_display_name(project_root),
                warnings: Vec::new(),
                fingerprint: unopened_fingerprint(project_root),
            });
        }
    };
    let layout = ProjectStorageLayout::new(project_root);
    let mut warnings = Vec::new();
    let ignore = read_gitignore(project_root).map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::FilesystemFailure,
    })?;
    if has_broad_usdhub_ignore(ignore.as_deref().unwrap_or_default()).map_err(|_| {
        ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::FilesystemFailure,
        }
    })? {
        warnings.push(ProjectInspectionWarning::BroadUsdHubIgnore);
    }
    if !layout.cache_dir().is_dir() || !layout.recovery_dir().is_dir() {
        warnings.push(ProjectInspectionWarning::MissingLocalCacheRoots);
    }

    let manifest_path = layout.manifest_path();
    let (classification, display_name) = match fs::read(&manifest_path) {
        Ok(bytes) => match serde_json::from_slice::<ProjectManifestV1>(&bytes) {
            Ok(manifest) if manifest.validate_schema_version().is_err() => {
                warnings.push(ProjectInspectionWarning::UnsupportedManifestVersion);
                (ProjectInspectionClassification::Incompatible, project_display_name(project_root))
            }
            Ok(manifest) if manifest.validate().is_ok() => (
                ProjectInspectionClassification::NativeUsdHub,
                manifest.name,
            ),
            _ => {
                warnings.push(ProjectInspectionWarning::MalformedManifest);
                (ProjectInspectionClassification::Incompatible, project_display_name(project_root))
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
            ProjectInspectionClassification::AdoptableGit,
            project_display_name(project_root),
        ),
        Err(_) => {
            warnings.push(ProjectInspectionWarning::MalformedManifest);
            (ProjectInspectionClassification::Incompatible, project_display_name(project_root))
        }
    };
    Ok(ProjectInspection {
        classification,
        display_name,
        warnings,
        fingerprint: repository_fingerprint(&repository, &layout)?,
    })
}

fn unopened_fingerprint(project_root: &Path) -> String {
    let mut hasher = blake3::Hasher::new();
    for relative in [".gitignore", ".usdhub/project.json"] {
        hasher.update(relative.as_bytes());
        hasher.update(&fs::read(project_root.join(relative)).unwrap_or_default());
    }
    hasher.finalize().to_hex().to_string()
}

fn repository_fingerprint(
    repository: &usd_git::Repository,
    layout: &ProjectStorageLayout,
) -> Result<String, ProjectWriteError> {
    use usd_git::GitRepository;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&fs::read(layout.manifest_path()).unwrap_or_default());
    hasher.update(&fs::read(layout.root().join(".gitignore")).unwrap_or_default());
    hasher.update(repository.current_branch().map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::RepositoryUnavailable,
    })?.unwrap_or_default().as_bytes());
    if let Some(head) = repository.head().map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::RepositoryUnavailable,
    })? {
        hasher.update(head.id().to_string().as_bytes());
    }
    for branch in repository.branches().map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::RepositoryUnavailable,
    })? {
        hasher.update(branch.name.as_bytes());
        hasher.update(branch.tip.to_string().as_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn project_summary(
    manifest: &ProjectManifestV1,
    project_root: &Path,
) -> Result<ProjectSummary, ProjectWriteError> {
    Ok(ProjectSummary {
        id: manifest.project_id,
        name: manifest.name.clone(),
        root: manifest.root.clone(),
        repository: repository_summary(manifest.project_id, project_root).map_err(|_| {
            ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::RepositoryUnavailable,
            }
        })?,
        counts: ProjectContentCounts {
            scenes: manifest.scenes.len() as u64,
            models: manifest.models.len() as u64,
            scene_placements: 0,
            model_placements: 0,
        },
        capabilities: ProjectCapabilities::default(),
    })
}

fn project_display_name(project_root: &Path) -> String {
    project_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Imported Project")
        .to_owned()
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
mod tests {
    use std::{collections::BTreeMap, fs};

    use tempfile::tempdir;
    use usd_git::GitRepository;

    use super::*;
    use crate::project::catalog::workspace_registry::WorkspaceRegistry;

    #[test]
    fn create_project_keeps_head_unborn_and_registers_last() {
        let directory = tempdir().unwrap();
        let parent = directory.path().join("projects");
        fs::create_dir(&parent).unwrap();
        fs::write(parent.join("keep.txt"), b"user data").unwrap();
        let registry_path = directory.path().join("workspace.json");
        let mut service = ProjectApplicationService::open(&registry_path).unwrap();

        let summary = service.create_project(&parent, "Created Project").unwrap();
        let project_root = parent.join("Created Project");
        let repository = usd_git::Repository::open(&project_root).unwrap();

        assert_eq!(summary.name, "Created Project");
        assert_eq!(repository.current_branch().unwrap().as_deref(), Some("main"));
        assert!(repository.head().unwrap().is_none());
        assert!(project_root.join(".git").is_dir());
        assert!(project_root.join(".usdhub/project.json").is_file());
        assert!(project_root.join(".usdhub/cache").is_dir());
        assert!(project_root.join(".usdhub/recovery").is_dir());
        assert_eq!(fs::read(parent.join("keep.txt")).unwrap(), b"user data");
        assert!(fs::read_to_string(project_root.join(".gitignore"))
            .unwrap()
            .contains(".usdhub/cache/"));
        assert_eq!(WorkspaceRegistry::load(registry_path)
            .unwrap()
            .get(summary.id)
            .unwrap()
            .repository_locator(), project_root);
    }

    #[test]
    fn create_project_rejects_unsafe_names_without_touching_parent() {
        let directory = tempdir().unwrap();
        let parent = directory.path().join("projects");
        fs::create_dir(&parent).unwrap();
        fs::write(parent.join("keep.txt"), b"user data").unwrap();
        let registry_path = directory.path().join("workspace.json");
        let mut service = ProjectApplicationService::open(registry_path).unwrap();

        for name in ["", ".", "..", "nested/name", "nested\\name", "bad\0name"] {
            assert!(matches!(
                service.create_project(&parent, name),
                Err(ProjectWriteError::Invalid {
                    code: ProjectWriteErrorCode::InvalidProjectName
                })
            ));
        }
        assert_eq!(fs::read(parent.join("keep.txt")).unwrap(), b"user data");
        assert_eq!(fs::read_dir(parent).unwrap().count(), 1);
    }

    #[test]
    fn import_inspection_is_read_only_and_classifies_adoptable_git() {
        let directory = tempdir().unwrap();
        let project_root = directory.path().join("existing");
        usd_git::Repository::init(&project_root).unwrap();
        fs::write(project_root.join("user.usda"), b"#usda 1.0\n").unwrap();
        let before = snapshot(&project_root);
        let service = ProjectApplicationService::open(directory.path().join("workspace.json"))
            .unwrap();

        let inspection = service.inspect_project(&project_root).unwrap();

        assert_eq!(
            inspection.classification,
            ProjectInspectionClassification::AdoptableGit
        );
        assert!(inspection
            .warnings
            .contains(&ProjectInspectionWarning::MissingLocalCacheRoots));
        assert_eq!(before, snapshot(&project_root));
    }

    #[test]
    fn native_project_with_deleted_local_state_remains_importable() {
        let directory = tempdir().unwrap();
        let parent = directory.path().join("projects");
        fs::create_dir(&parent).unwrap();
        let registry_path = directory.path().join("workspace.json");
        let mut service = ProjectApplicationService::open(&registry_path).unwrap();
        let summary = service.create_project(&parent, "Native").unwrap();
        let project_root = parent.join("Native");
        fs::remove_dir_all(project_root.join(".usdhub/cache")).unwrap();
        fs::remove_dir_all(project_root.join(".usdhub/recovery")).unwrap();

        let inspection = service.inspect_project(&project_root).unwrap();

        assert_eq!(
            inspection.classification,
            ProjectInspectionClassification::NativeUsdHub
        );
        assert!(inspection
            .warnings
            .contains(&ProjectInspectionWarning::MissingLocalCacheRoots));
        assert_eq!(inspection.display_name, summary.name);
        assert_eq!(fs::read_dir(project_root.join(".usdhub")).unwrap().count(), 1);
    }

    fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
        fn visit(root: &Path, current: &Path, output: &mut BTreeMap<String, Vec<u8>>) {
            let mut entries = fs::read_dir(current)
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                let relative = path.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                if path.is_dir() {
                    visit(root, &path, output);
                } else {
                    output.insert(relative, fs::read(path).unwrap());
                }
            }
        }

        let mut output = BTreeMap::new();
        visit(root, root, &mut output);
        output
    }
}

impl CreateProjectJournal {
    fn rollback(&self, project_root: &Path) {
        if self.project_dir_created && !self.registry_entry_added {
            let _ = fs::remove_dir_all(project_root);
        }
    }
}
