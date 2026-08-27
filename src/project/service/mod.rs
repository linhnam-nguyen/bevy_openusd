//! Read-only Project application facade.
//!
//! This is the only backend entry point needed by the M13 native host. It
//! resolves a stable ProjectId through the private workspace registry and
//! returns owned, Git-neutral protocol DTOs.

use std::{path::Path, path::PathBuf};

use project_protocol::{
    ProjectListItem, ProjectReadCommand, ProjectReadError, ProjectReadErrorCode, ProjectReadReply,
    ProjectReadRequest, ProjectReadResponse,
};
use usd_project::{
    BranchSummary, ModelSourceSummary, ProjectContentNode, ProjectId, RepositorySummary,
    RevisionSummary,
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
}

mod lifecycle;

impl ProjectApplicationService {
    /// Open the host-owned workspace registry without exposing its locator.
    pub fn open(registry_path: impl Into<PathBuf>) -> Result<Self, ProjectReadError> {
        WorkspaceRegistry::load(registry_path)
            .map(|registry| Self { registry })
            .map_err(|_| ProjectReadError::HostUnavailable {
                code: ProjectReadErrorCode::RegistryUnavailable,
            })
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
                Ok(ProjectReadResponse::ProjectTree {
                    project_id: *project_id,
                    nodes: project_tree(entry.repository_locator(), &manifest)?,
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
) -> Result<Vec<ProjectContentNode>, ProjectReadError> {
    let mut scenes = manifest.scenes().to_vec();
    scenes.sort_by_key(|scene| scene.id);
    let mut models = manifest.models().to_vec();
    models.sort_by_key(|model| model.id);

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
                    nodes.push(ProjectContentNode::ScenePlacement {
                        member_id: member.id,
                        target,
                        parent_scene_id: scene.id,
                        name: member.name,
                    });
                }
                usd_project::SceneMemberTarget::Model(target) => {
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
    Ok(nodes)
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
    Ok(RepositorySummary {
        active_branch,
        branches,
        dirty,
        head: head.map(|revision| RevisionSummary {
            id: revision.id().to_string(),
        }),
    })
}

#[cfg(test)]
#[path = "repository_summary_tests.rs"]
mod repository_summary_tests;

#[cfg(test)]
mod tests {
    use project_protocol::{ProjectReadError, ProjectReadResponse};
    use tempfile::tempdir;
    use usd_project::{
        ModelId, ModelManifestEntry, ModelSourceKind, ProjectManifestV1, ProjectRoot, SceneId,
        SceneManifestEntry, SceneMember, SceneMemberId, SceneMemberTarget, StorageKey,
    };

    use super::*;
    use crate::project::catalog::{
        manifest_store::ManifestStore, workspace_registry::WorkspaceRegistry,
    };

    #[test]
    fn unknown_project_id_returns_typed_not_found_without_a_path() {
        let directory = tempdir().unwrap();
        let registry = WorkspaceRegistry::load(directory.path().join("workspace.json")).unwrap();
        let service = ProjectApplicationService { registry };
        let project_id = ProjectId::new_v4();

        let reply = service.execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
            project_id,
        )));

        assert_eq!(reply.result, Err(ProjectReadError::NotFound { project_id }));
        assert!(!format!("{reply:?}").contains(directory.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn list_projects_returns_owned_summaries_from_the_registry() {
        let directory = tempdir().unwrap();
        let registry_path = directory.path().join("workspace.json");
        let project_id = ProjectId::new_v4();
        let repository = directory.path().join("repository");
        let manifest = ProjectManifestV1::new(
            project_id,
            "Project",
            ProjectRoot::Empty,
            Vec::new(),
            Vec::new(),
        );
        ManifestStore::write_manifest_atomic(&repository, &manifest).unwrap();
        let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
        registry.register(project_id, &repository, None).unwrap();
        let service = ProjectApplicationService { registry };

        let reply = service.execute(ProjectReadCommand::new(ProjectReadRequest::ListProjects));
        let ProjectReadResponse::Projects(items) = reply.result.unwrap() else {
            panic!("ListProjects must return catalogue items");
        };
        assert!(matches!(items.as_slice(), [ProjectListItem::Available(_)]));
    }

    #[test]
    fn tree_projection_keeps_authored_model_placement_identity() {
        let directory = tempdir().unwrap();
        let registry_path = directory.path().join("workspace.json");
        let project_id = ProjectId::new_v4();
        let scene_id = SceneId::new_v4();
        let model_id = ModelId::new_v4();
        let member_id = SceneMemberId::new_v4();
        let repository = directory.path().join("repository");
        let manifest = ProjectManifestV1::new(
            project_id,
            "Project",
            ProjectRoot::Scene(scene_id),
            vec![SceneManifestEntry {
                id: scene_id,
                storage_key: StorageKey::new("scene").unwrap(),
            }],
            vec![ModelManifestEntry {
                id: model_id,
                source_kind: ModelSourceKind::Usd,
                storage_key: StorageKey::new("model").unwrap(),
            }],
        );
        ManifestStore::write_manifest_atomic(&repository, &manifest).unwrap();
        crate::project::scene::authoring::author_scene_atomic_with_members(
            &repository,
            scene_id,
            &[SceneMember {
                id: member_id,
                target: SceneMemberTarget::Model(model_id),
                name: Some("Placed model".to_owned()),
            }],
        )
        .unwrap();
        let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
        registry.register(project_id, &repository, None).unwrap();
        let service = ProjectApplicationService { registry };

        let reply = service.execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
            project_id,
        )));
        let ProjectReadResponse::ProjectTree { nodes, .. } = reply.result.unwrap() else {
            panic!("GetProjectTree must return ProjectTree");
        };

        assert!(nodes.iter().any(|node| {
            matches!(
                node,
                ProjectContentNode::ModelPlacement {
                    member_id: id,
                    target,
                    parent_scene_id,
                    ..
                } if *id == member_id && *target == model_id && *parent_scene_id == scene_id
            )
        }));
    }
}
