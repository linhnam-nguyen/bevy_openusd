//! Read-only Project application facade.
//!
//! This is the only backend entry point needed by the M13 native host. It
//! resolves a stable ProjectId through the private workspace registry and
//! returns owned, Git-neutral protocol DTOs.

use std::{
    collections::HashMap,
    path::Path,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use project_protocol::{
    ProjectListItem, ProjectReadCommand, ProjectReadError, ProjectReadErrorCode, ProjectReadReply,
    ProjectReadRequest, ProjectReadResponse,
};
use usd_project::{
    BranchSummary, CommitSummary, ModelSourceSummary, ProjectContentNode, ProjectId,
    RepositorySummary, RevisionSummary,
};

use crate::project::{
    catalog::{
        catalogue::{ProjectCatalogueItem, ProjectCatalogueUnavailableReason, list_projects},
        manifest_store::ManifestStore,
        workspace_registry::{WorkspaceProjectEntry, WorkspaceRegistry},
    },
    scene::authoring::{read_scene_members, scene_path},
};

/// Read-only Project application service owned by the backend boundary.
pub struct ProjectApplicationService {
    registry: WorkspaceRegistry,
    pub(super) publication_coordinator: ProjectPublicationCoordinator,
    pub(super) stage_mutations: ProjectStageMutationQueue,
    pub(super) progress: ProjectImportProgressStore,
}

/// Shared admission state for non-idempotent publication mutations.
///
/// The host may construct a fresh application service for each command, so
/// this coordinator must outlive an individual request. The map lock is held
/// only while resolving a Project-specific lock; publication work is serialized
/// by the returned lock and never by one global Project mutex.
#[derive(Clone, Default)]
pub struct ProjectPublicationCoordinator {
    publishers: Arc<Mutex<HashMap<ProjectId, Arc<Mutex<()>>>>>,
}

impl ProjectPublicationCoordinator {
    pub fn publisher(&self, project_id: ProjectId) -> Arc<Mutex<()>> {
        self.publishers
            .lock()
            .expect("Project publication coordinator is not poisoned")
            .entry(project_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }
}

mod inspection;
mod lifecycle;
mod model;
mod model_preparation;
mod progress;
mod scene;
mod scene_adoption;
mod stage_mutation;
pub use model_preparation::ProjectModelPreparationQueue;
pub use progress::ProjectImportProgressStore;
pub use scene_inspection::ProjectSceneInspectionQueue;
pub use stage_mutation::{ProjectStageMutation, ProjectStageMutationQueue};
mod scene_inspection;

impl ProjectApplicationService {
    /// Open the host-owned workspace registry without exposing its locator.
    pub fn open(registry_path: impl Into<PathBuf>) -> Result<Self, ProjectReadError> {
        Self::open_with_publication_coordinator(
            registry_path,
            ProjectPublicationCoordinator::default(),
        )
    }

    /// Open the service with host-owned shared publication admission state.
    pub fn open_with_publication_coordinator(
        registry_path: impl Into<PathBuf>,
        publication_coordinator: ProjectPublicationCoordinator,
    ) -> Result<Self, ProjectReadError> {
        Self::open_with_project_state(
            registry_path,
            publication_coordinator,
            ProjectStageMutationQueue::default(),
        )
    }

    /// Open the service with all host-owned shared Project state.
    pub fn open_with_project_state(
        registry_path: impl Into<PathBuf>,
        publication_coordinator: ProjectPublicationCoordinator,
        stage_mutations: ProjectStageMutationQueue,
    ) -> Result<Self, ProjectReadError> {
        Self::open_with_project_state_and_progress(
            registry_path,
            publication_coordinator,
            stage_mutations,
            ProjectImportProgressStore::default(),
        )
    }

    /// Open the service with all host-owned shared Project state, including
    /// the progress status observed by the native host.
    pub fn open_with_project_state_and_progress(
        registry_path: impl Into<PathBuf>,
        publication_coordinator: ProjectPublicationCoordinator,
        stage_mutations: ProjectStageMutationQueue,
        progress: ProjectImportProgressStore,
    ) -> Result<Self, ProjectReadError> {
        WorkspaceRegistry::load(registry_path)
            .map(|registry| Self {
                registry,
                publication_coordinator,
                stage_mutations,
                progress,
            })
            .map_err(|_| ProjectReadError::HostUnavailable {
                code: ProjectReadErrorCode::RegistryUnavailable,
            })
    }

    /// Open with a shared LiveStage handoff queue and default publication
    /// admission. This is useful for hosts that already own stage activation.
    pub fn open_with_stage_mutation_queue(
        registry_path: impl Into<PathBuf>,
        stage_mutations: ProjectStageMutationQueue,
    ) -> Result<Self, ProjectReadError> {
        Self::open_with_project_state(
            registry_path,
            ProjectPublicationCoordinator::default(),
            stage_mutations,
        )
    }

    /// Execute one versioned read command and return a typed reply envelope.
    pub fn execute(&self, command: ProjectReadCommand) -> ProjectReadReply {
        match command.validate() {
            Ok(()) => match self.read(&command.request) {
                Ok(response) => ProjectReadReply::success(response),
                Err(error) => ProjectReadReply::failure(error),
            },
            Err(error) => ProjectReadReply::failure(error),
        }
    }

    fn read(&self, request: &ProjectReadRequest) -> Result<ProjectReadResponse, ProjectReadError> {
        match request {
            ProjectReadRequest::ListProjects => Ok(ProjectReadResponse::Projects(
                list_projects(&self.registry)
                    .into_iter()
                    .map(project_list_item)
                    .collect(),
            )),
            ProjectReadRequest::GetProjectTree(project_id) => {
                let (entry, manifest) = self.validated_project(*project_id)?;
                let (nodes, counts) = project_tree(entry.repository_locator(), &manifest)?;
                Ok(ProjectReadResponse::ProjectTree {
                    project_id: *project_id,
                    nodes,
                    counts,
                })
            }
            ProjectReadRequest::GetProjectRepositorySummary(project_id) => {
                let (entry, _) = self.validated_project(*project_id)?;
                Ok(ProjectReadResponse::RepositorySummary {
                    project_id: *project_id,
                    repository: repository_summary(*project_id, entry.repository_locator())?,
                })
            }
        }
    }

    fn validated_project(
        &self,
        project_id: ProjectId,
    ) -> Result<
        (
            &WorkspaceProjectEntry,
            usd_project::ValidatedProjectManifest,
        ),
        ProjectReadError,
    > {
        let entry = self
            .registry
            .get(project_id)
            .ok_or(ProjectReadError::NotFound { project_id })?;
        let manifest = ManifestStore::read_validated(entry.repository_locator()).map_err(|_| {
            ProjectReadError::Unavailable {
                project_id,
                code: ProjectReadErrorCode::ManifestUnavailable,
            }
        })?;
        if manifest.raw().project_id != project_id {
            return Err(ProjectReadError::Unavailable {
                project_id,
                code: ProjectReadErrorCode::RegistryIdentityMismatch,
            });
        }
        Ok((entry, manifest))
    }
}

fn project_list_item(item: ProjectCatalogueItem) -> ProjectListItem {
    match item {
        ProjectCatalogueItem::Available(summary) => ProjectListItem::Available(summary),
        ProjectCatalogueItem::Unavailable { project_id, reason } => ProjectListItem::Unavailable {
            project_id,
            code: match reason {
                ProjectCatalogueUnavailableReason::ManifestUnavailable => {
                    ProjectReadErrorCode::ManifestUnavailable
                }
                ProjectCatalogueUnavailableReason::RegistryIdentityMismatch => {
                    ProjectReadErrorCode::RegistryIdentityMismatch
                }
            },
        },
    }
}

fn project_tree(
    project_root: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
) -> Result<(Vec<ProjectContentNode>, usd_project::ProjectContentCounts), ProjectReadError> {
    let mut scenes = manifest.scenes().to_vec();
    scenes.sort_by_key(|scene| scene.id);
    let mut models = manifest.models().to_vec();
    models.sort_by_key(|model| model.id);

    let mut counts = usd_project::ProjectContentCounts {
        scenes: scenes.len() as u64,
        models: models.len() as u64,
        ..Default::default()
    };
    let mut nodes = Vec::with_capacity(scenes.len() + models.len());
    nodes.extend(scenes.iter().map(|scene| ProjectContentNode::Scene {
        scene_id: scene.id,
        name: scene.storage_key.to_string(),
    }));
    nodes.extend(models.iter().map(|model| ProjectContentNode::Model {
        model_id: model.id,
        name: model.storage_key.to_string(),
        source: ModelSourceSummary {
            kind: model.source_kind.clone(),
        },
    }));

    for scene in scenes {
        let members =
            read_scene_members(&scene_path(project_root, scene.id), scene.id).map_err(|_| {
                ProjectReadError::Unavailable {
                    project_id: manifest.raw().project_id,
                    code: ProjectReadErrorCode::InvalidProjectData,
                }
            })?;
        for member in members {
            match member.target {
                usd_project::SceneMemberTarget::Scene(target) => {
                    counts.scene_placements += 1;
                    nodes.push(ProjectContentNode::ScenePlacement {
                        member_id: member.id,
                        target,
                        parent_scene_id: scene.id,
                        name: member.name,
                    });
                }
                usd_project::SceneMemberTarget::Model(target) => {
                    counts.model_placements += 1;
                    nodes.push(ProjectContentNode::ModelPlacement {
                        member_id: member.id,
                        target,
                        parent_scene_id: scene.id,
                        name: member.name,
                    });
                }
            }
        }
    }
    Ok((nodes, counts))
}

fn repository_summary(
    project_id: ProjectId,
    project_root: &Path,
) -> Result<RepositorySummary, ProjectReadError> {
    use usd_git::{GitRepository, Repository};

    let repository = Repository::open(project_root).map_err(|_| ProjectReadError::Unavailable {
        project_id,
        code: ProjectReadErrorCode::RepositoryUnavailable,
    })?;
    let active_branch = repository
        .current_branch()
        .map_err(|_| ProjectReadError::Unavailable {
            project_id,
            code: ProjectReadErrorCode::RepositoryUnavailable,
        })?;
    let head = repository
        .head()
        .map_err(|_| ProjectReadError::Unavailable {
            project_id,
            code: ProjectReadErrorCode::RepositoryUnavailable,
        })?;
    let branches = repository
        .branches()
        .map_err(|_| ProjectReadError::Unavailable {
            project_id,
            code: ProjectReadErrorCode::RepositoryUnavailable,
        })?
        .into_iter()
        .map(|branch| BranchSummary {
            name: branch.name,
            tip: RevisionSummary {
                id: branch.tip.to_string(),
            },
            is_current: branch.is_current,
        })
        .collect::<Vec<_>>();
    let dirty = repository
        .is_dirty()
        .map_err(|_| ProjectReadError::Unavailable {
            project_id,
            code: ProjectReadErrorCode::RepositoryUnavailable,
        })?;
    let latest_commit = head
        .as_ref()
        .map(|revision| {
            repository
                .read_commit(revision.id())
                .map(|commit| CommitSummary {
                    revision: RevisionSummary {
                        id: commit.id.to_string(),
                    },
                    subject: commit
                        .message
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .to_owned(),
                    author: commit.author.name,
                    authored_at_seconds: commit.author.time_seconds,
                })
        })
        .transpose()
        .map_err(|_| ProjectReadError::Unavailable {
            project_id,
            code: ProjectReadErrorCode::RepositoryUnavailable,
        })?;
    Ok(RepositorySummary {
        active_branch,
        branches,
        dirty,
        head: head.map(|revision| RevisionSummary {
            id: revision.id().to_string(),
        }),
        latest_commit,
    })
}

#[cfg(test)]
#[path = "repository_summary_tests.rs"]
mod repository_summary_tests;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
