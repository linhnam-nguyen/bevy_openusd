use std::{cmp::Ordering, fs, io::ErrorKind};

use usd_project::{
    ProjectCapabilities, ProjectContentCounts, ProjectId, ProjectSummary, RepositorySummary,
};

use super::{
    manifest_store::ManifestStore,
    workspace_registry::{WorkspaceProjectEntry, WorkspaceRegistry},
};
use crate::project::storage::ProjectStorageLayout;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProjectCatalogueUnavailableReason {
    ManifestUnavailable,
    RepositoryMissing,
    RepositoryPermissionDenied,
    InvalidManifest,
    RegistryIdentityMismatch,
}

/// One cheap catalogue result without exposing a machine-local repository path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProjectCatalogueItem {
    Available(ProjectSummary),
    Unavailable {
        project_id: ProjectId,
        reason: ProjectCatalogueUnavailableReason,
    },
}

/// List registered Projects from metadata manifests without opening USD stages.
pub(crate) fn list_projects(registry: &WorkspaceRegistry) -> Vec<ProjectCatalogueItem> {
    let mut items = Vec::with_capacity(registry.entries().len());
    for entry in registry.entries() {
        items.push(catalogue_item(entry));
    }
    items.sort_by(compare_items);
    items
}

fn catalogue_item(entry: &WorkspaceProjectEntry) -> ProjectCatalogueItem {
    match ManifestStore::read_validated(entry.repository_locator()) {
        Ok(manifest) if manifest.raw().project_id != entry.content_project_id() => {
            ProjectCatalogueItem::Unavailable {
                project_id: entry.project_id(),
                reason: ProjectCatalogueUnavailableReason::RegistryIdentityMismatch,
            }
        }
        Ok(manifest) => ProjectCatalogueItem::Available(ProjectSummary {
            id: entry.project_id(),
            name: manifest.raw().name.clone(),
            root: manifest.raw().root.clone(),
            repository: RepositorySummary {
                active_branch: None,
                branches: Vec::new(),
                dirty: false,
                head: None,
                latest_commit: None,
            },
            counts: ProjectContentCounts {
                scenes: manifest.scenes().len() as u64,
                models: manifest.models().len() as u64,
                scene_placements: 0,
                model_placements: 0,
            },
            issues: usd_project::ProjectIssueSummary::default(),
            people: usd_project::ProjectPeopleSummary::default(),
            capabilities: ProjectCapabilities::default(),
        }),
        Err(_) => ProjectCatalogueItem::Unavailable {
            project_id: entry.project_id(),
            reason: unavailable_reason(entry),
        },
    }
}

pub(crate) fn unavailable_reason(
    entry: &WorkspaceProjectEntry,
) -> ProjectCatalogueUnavailableReason {
    let root = entry.repository_locator();
    match fs::metadata(root) {
        Err(error) if error.kind() == ErrorKind::NotFound => {
            ProjectCatalogueUnavailableReason::RepositoryMissing
        }
        Err(error) if error.kind() == ErrorKind::PermissionDenied => {
            ProjectCatalogueUnavailableReason::RepositoryPermissionDenied
        }
        Err(_) => ProjectCatalogueUnavailableReason::ManifestUnavailable,
        Ok(metadata) if !metadata.is_dir() => ProjectCatalogueUnavailableReason::InvalidManifest,
        Ok(_) => match fs::metadata(ProjectStorageLayout::new(root).readable_manifest_path()) {
            Err(error) if error.kind() == ErrorKind::NotFound => {
                ProjectCatalogueUnavailableReason::ManifestUnavailable
            }
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                ProjectCatalogueUnavailableReason::RepositoryPermissionDenied
            }
            Err(_) => ProjectCatalogueUnavailableReason::InvalidManifest,
            Ok(metadata) if !metadata.is_file() => {
                ProjectCatalogueUnavailableReason::InvalidManifest
            }
            Ok(_) => ProjectCatalogueUnavailableReason::InvalidManifest,
        },
    }
}

fn compare_items(left: &ProjectCatalogueItem, right: &ProjectCatalogueItem) -> Ordering {
    match (left, right) {
        (ProjectCatalogueItem::Available(left), ProjectCatalogueItem::Available(right)) => left
            .name
            .cmp(&right.name)
            .then_with(|| left.id.cmp(&right.id)),
        (
            ProjectCatalogueItem::Unavailable {
                project_id: left, ..
            },
            ProjectCatalogueItem::Unavailable {
                project_id: right, ..
            },
        ) => left.cmp(right),
        (ProjectCatalogueItem::Available(_), ProjectCatalogueItem::Unavailable { .. }) => {
            Ordering::Less
        }
        (ProjectCatalogueItem::Unavailable { .. }, ProjectCatalogueItem::Available(_)) => {
            Ordering::Greater
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/project/catalog/catalogue_tests.rs"]
mod tests;
