use std::{fs, path::Path};

use project_protocol::{
    ProjectInspection, ProjectInspectionClassification, ProjectInspectionWarning,
    ProjectWriteError, ProjectWriteErrorCode,
};
use usd_git::GitRepository;
use usd_project::{ProjectCapabilities, ProjectContentCounts, ProjectManifestV1, ProjectSummary};

use super::repository_summary;
use crate::project::storage::{ProjectStorageLayout, has_broad_usdhub_ignore, read_gitignore};

pub(super) fn inspect_project(project_root: &Path) -> Result<ProjectInspection, ProjectWriteError> {
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
    let tracked_cache = repository
        .has_tracked_path_prefix(".usdhub/cache")
        .map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::RepositoryUnavailable,
        })?;
    let tracked_recovery = repository
        .has_tracked_path_prefix(".usdhub/recovery")
        .map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::RepositoryUnavailable,
        })?;
    if tracked_cache || tracked_recovery {
        warnings.push(ProjectInspectionWarning::TrackedDerivedLocalState);
    }

    let manifest_path = layout.manifest_path();
    let (classification, display_name) = match fs::read(&manifest_path) {
        Ok(bytes) => match serde_json::from_slice::<ProjectManifestV1>(&bytes) {
            Ok(manifest) if manifest.validate_schema_version().is_err() => {
                warnings.push(ProjectInspectionWarning::UnsupportedManifestVersion);
                (
                    ProjectInspectionClassification::Incompatible,
                    project_display_name(project_root),
                )
            }
            Ok(manifest) if manifest.validate().is_ok() => {
                (ProjectInspectionClassification::NativeUsdHub, manifest.name)
            }
            _ => {
                warnings.push(ProjectInspectionWarning::MalformedManifest);
                (
                    ProjectInspectionClassification::Incompatible,
                    project_display_name(project_root),
                )
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
            ProjectInspectionClassification::AdoptableGit,
            project_display_name(project_root),
        ),
        Err(_) => {
            warnings.push(ProjectInspectionWarning::MalformedManifest);
            (
                ProjectInspectionClassification::Incompatible,
                project_display_name(project_root),
            )
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
    let mut hasher = blake3::Hasher::new();
    hasher.update(&fs::read(layout.manifest_path()).unwrap_or_default());
    hasher.update(&fs::read(layout.root().join(".gitignore")).unwrap_or_default());
    hasher.update(
        repository
            .current_branch()
            .map_err(|_| ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::RepositoryUnavailable,
            })?
            .unwrap_or_default()
            .as_bytes(),
    );
    if let Some(head) = repository.head().map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::RepositoryUnavailable,
    })? {
        hasher.update(head.id().to_string().as_bytes());
    }
    for branch in repository
        .branches()
        .map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::RepositoryUnavailable,
        })?
    {
        hasher.update(branch.name.as_bytes());
        hasher.update(branch.tip.to_string().as_bytes());
    }
    for (prefix, tracked) in [
        (
            ".usdhub/cache",
            repository
                .has_tracked_path_prefix(".usdhub/cache")
                .map_err(|_| ProjectWriteError::Failed {
                    code: ProjectWriteErrorCode::RepositoryUnavailable,
                })?,
        ),
        (
            ".usdhub/recovery",
            repository
                .has_tracked_path_prefix(".usdhub/recovery")
                .map_err(|_| ProjectWriteError::Failed {
                    code: ProjectWriteErrorCode::RepositoryUnavailable,
                })?,
        ),
    ] {
        hasher.update(prefix.as_bytes());
        hasher.update(&[tracked as u8]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

pub(super) fn project_summary(
    manifest: &ProjectManifestV1,
    project_root: &Path,
) -> Result<ProjectSummary, ProjectWriteError> {
    let validated = manifest
        .validate_and_index()
        .map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::ManifestUnavailable,
        })?;
    let (_, counts) =
        super::project_tree(project_root, &validated).map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::FilesystemFailure,
        })?;
    Ok(ProjectSummary {
        id: manifest.project_id,
        name: manifest.name.clone(),
        root: manifest.root.clone(),
        repository: repository_summary(manifest.project_id, project_root).map_err(|_| {
            ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::RepositoryUnavailable,
            }
        })?,
        counts,
        issues: usd_project::ProjectIssueSummary::default(),
        people: usd_project::ProjectPeopleSummary::default(),
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
