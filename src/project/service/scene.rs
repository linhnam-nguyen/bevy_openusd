//! Application service for authoritative Scene creation.

use std::path::Path;

use project_protocol::{
    ProjectSceneWriteResponse, ProjectWriteError, ProjectWriteErrorCode, ProjectWriteTarget,
};
use usd_project::{ProjectRoot, SceneCompositionGraph, SceneMemberTarget};

use super::ProjectApplicationService;

pub(super) fn create_scene(
    service: &mut ProjectApplicationService,
    project_id: usd_project::ProjectId,
    target: ProjectWriteTarget,
    name: &str,
) -> Result<ProjectSceneWriteResponse, ProjectWriteError> {
    let (entry, validated) =
        service
            .validated_project(project_id)
            .map_err(|error| ProjectWriteError::Failed {
                code: match error {
                    project_protocol::ProjectReadError::NotFound { .. } => {
                        ProjectWriteErrorCode::SelectionUnavailable
                    }
                    _ => ProjectWriteErrorCode::ManifestUnavailable,
                },
            })?;
    let name = name.trim();
    let storage_key =
        usd_project::StorageKey::new(name.to_owned()).map_err(|_| ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::InvalidSceneName,
        })?;
    if validated
        .scenes()
        .iter()
        .any(|scene| scene.storage_key == storage_key)
        || validated
            .models()
            .iter()
            .any(|model| model.storage_key == storage_key)
    {
        return Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::InvalidSceneName,
        });
    }

    let parent_scene_id = match target {
        ProjectWriteTarget::Project(target_project_id) if target_project_id == project_id => {
            match validated.raw().root {
                ProjectRoot::Empty => None,
                ProjectRoot::Scene(scene_id) => Some(scene_id),
                ProjectRoot::Model(_) => {
                    return Err(ProjectWriteError::Invalid {
                        code: ProjectWriteErrorCode::InvalidRootForComposition,
                    });
                }
            }
        }
        ProjectWriteTarget::Project(_) => {
            return Err(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::InvalidSelection,
            });
        }
        ProjectWriteTarget::Scene(scene_id) => {
            if validated.scene(scene_id).is_none() {
                return Err(ProjectWriteError::Invalid {
                    code: ProjectWriteErrorCode::InvalidSelection,
                });
            }
            Some(scene_id)
        }
        ProjectWriteTarget::Model(_) => {
            return Err(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::InvalidRootForComposition,
            });
        }
    };

    let project_root = entry.repository_locator();
    let graph = scene_graph(project_root, &validated).map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::FilesystemFailure,
    })?;
    let parent_members = parent_scene_id
        .map(|scene_id| {
            crate::project::scene::authoring::read_scene_members(
                &crate::project::scene::authoring::scene_path(project_root, scene_id),
                scene_id,
            )
        })
        .transpose()
        .map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::FilesystemFailure,
        })?
        .unwrap_or_default();
    if parent_scene_id.is_some() {
        service.stage_mutations.ensure_capacity(project_root)?;
    }

    let created = crate::project::scene::create::create_scene_atomic(
        crate::project::scene::create::CreateSceneRequest {
            project_root,
            base_manifest: validated.raw(),
            graph: &graph,
            parent_scene_id,
            parent_members: &parent_members,
            storage_key,
            set_as_root: parent_scene_id.is_none(),
        },
    )
    .map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::FilesystemFailure,
    })?;
    let summary = super::inspection::project_summary(&created.manifest, project_root)?;
    if let Some(parent_scene_id) = parent_scene_id {
        service.stage_mutations.submit_for_project(
            project_root,
            super::ProjectStageMutation::CreateScene {
                project_id,
                scene_id: created.scene_id,
                parent_scene_id: Some(parent_scene_id),
                placement_id: created.member.as_ref().map(|member| member.id),
            },
        )?;
    }
    let _ = service.cache_warm.enqueue(
        project_root,
        crate::project::cache::ProjectCacheTarget::Scene {
            id: created.scene_id.to_string(),
        },
    );
    Ok(ProjectSceneWriteResponse {
        project: summary,
        scene_id: created.scene_id,
        placement_id: created.member.map(|member| member.id),
    })
}

pub(super) fn scene_graph(
    project_root: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
) -> anyhow::Result<SceneCompositionGraph> {
    let mut edges = Vec::new();
    for scene in manifest.scenes() {
        for member in crate::project::scene::authoring::read_scene_members(
            &crate::project::scene::authoring::scene_path(project_root, scene.id),
            scene.id,
        )? {
            if let SceneMemberTarget::Scene(target) = member.target {
                edges.push((scene.id, target));
            }
        }
    }
    Ok(SceneCompositionGraph::from_edges(edges))
}
