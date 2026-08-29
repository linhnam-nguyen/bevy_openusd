use std::path::Path;

use project_protocol::{ProjectListItem, ProjectReadError, ProjectReadErrorCode};
use usd_project::{
    BranchSummary, CommitSummary, ModelSourceSummary, ProjectContentNode, ProjectId,
    RepositorySummary, RevisionSummary,
};

use crate::project::catalog::catalogue::{ProjectCatalogueItem, ProjectCatalogueUnavailableReason};
use crate::project::scene::authoring::{read_scene_members, scene_path};

pub(super) fn project_list_item(item: ProjectCatalogueItem) -> ProjectListItem {
    match item {
        ProjectCatalogueItem::Available(summary) => ProjectListItem::Available(Box::new(summary)),
        ProjectCatalogueItem::Unavailable { project_id, reason } => ProjectListItem::Unavailable {
            project_id,
            code: match reason {
                ProjectCatalogueUnavailableReason::ManifestUnavailable => {
                    ProjectReadErrorCode::ManifestUnavailable
                }
                ProjectCatalogueUnavailableReason::RepositoryMissing => {
                    ProjectReadErrorCode::RepositoryMissing
                }
                ProjectCatalogueUnavailableReason::RepositoryPermissionDenied => {
                    ProjectReadErrorCode::RepositoryPermissionDenied
                }
                ProjectCatalogueUnavailableReason::InvalidManifest => {
                    ProjectReadErrorCode::InvalidManifest
                }
                ProjectCatalogueUnavailableReason::RegistryIdentityMismatch => {
                    ProjectReadErrorCode::RegistryIdentityMismatch
                }
            },
        },
    }
}

pub(super) fn project_tree(
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

pub(super) fn repository_summary(
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
