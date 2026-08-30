//! Application service for authoritative composed Scene adoption.

use std::path::Path;

use project_protocol::{
    PlacementSpec, ProjectImportPhase, ProjectImportProgress, ProjectSceneAdoptionResponse,
    ProjectWriteError, ProjectWriteErrorCode, ProjectWriteTarget,
};
use usd_project::{CompositionInspection, ProjectRoot, SceneMember};

use super::ProjectApplicationService;

pub(super) fn adopt_scene(
    service: &mut ProjectApplicationService,
    project_id: usd_project::ProjectId,
    target: ProjectWriteTarget,
    source: &Path,
    inspection: &CompositionInspection,
    name: String,
    operation_id: String,
    generation: u64,
    placement: PlacementSpec,
) -> Result<ProjectSceneAdoptionResponse, ProjectWriteError> {
    service.progress.publish(ProjectImportProgress {
        operation_id: operation_id.clone(),
        generation,
        phase: ProjectImportPhase::Queued,
    });
    service.progress.publish(ProjectImportProgress {
        operation_id: operation_id.clone(),
        generation,
        phase: ProjectImportPhase::Inspecting,
    });
    let result = adopt_scene_inner(
        service,
        project_id,
        target,
        source,
        inspection,
        name,
        operation_id.clone(),
        generation,
        placement,
        None,
    );
    service.progress.publish(ProjectImportProgress {
        operation_id,
        generation,
        phase: if result.is_ok() {
            ProjectImportPhase::Completed
        } else {
            ProjectImportPhase::Failed
        },
    });
    result
}

pub(super) fn link_scene(
    service: &mut ProjectApplicationService,
    project_id: usd_project::ProjectId,
    target: ProjectWriteTarget,
    source: &Path,
    inspection: &CompositionInspection,
    name: String,
    operation_id: String,
    generation: u64,
    placement: PlacementSpec,
) -> Result<ProjectSceneAdoptionResponse, ProjectWriteError> {
    service.progress.publish(ProjectImportProgress {
        operation_id: operation_id.clone(),
        generation,
        phase: ProjectImportPhase::Queued,
    });
    let result = adopt_scene_inner(
        service,
        project_id,
        target,
        source,
        inspection,
        name,
        operation_id.clone(),
        generation,
        placement,
        Some(source),
    );
    service.progress.publish(ProjectImportProgress {
        operation_id,
        generation,
        phase: if result.is_ok() {
            ProjectImportPhase::Completed
        } else {
            ProjectImportPhase::Failed
        },
    });
    result
}

pub(super) fn sync_linked_scene(
    service: &mut ProjectApplicationService,
    project_id: usd_project::ProjectId,
    scene_id: usd_project::SceneId,
    operation_id: String,
    generation: u64,
) -> Result<ProjectSceneAdoptionResponse, ProjectWriteError> {
    service.progress.publish(ProjectImportProgress {
        operation_id: operation_id.clone(),
        generation,
        phase: ProjectImportPhase::Queued,
    });
    let result =
        (|| {
            service.progress.publish(ProjectImportProgress {
                operation_id: operation_id.clone(),
                generation,
                phase: ProjectImportPhase::Validating,
            });
            let (entry, validated) = service.validated_project(project_id).map_err(|error| {
                ProjectWriteError::Failed {
                    code: match error {
                        project_protocol::ProjectReadError::NotFound { .. } => {
                            ProjectWriteErrorCode::SelectionUnavailable
                        }
                        _ => ProjectWriteErrorCode::ManifestUnavailable,
                    },
                }
            })?;
            let project_root = entry.repository_locator();
            validated
                .scene(scene_id)
                .ok_or(ProjectWriteError::Invalid {
                    code: ProjectWriteErrorCode::SceneNotFound,
                })?;
            let source =
                crate::project::link::resolve_source(project_root, scene_id).map_err(|_| {
                    ProjectWriteError::Failed {
                        code: ProjectWriteErrorCode::FilesystemFailure,
                    }
                })?;
            let inspection = crate::project::scene::inspection::inspect_composition(&source)
                .map_err(|_| ProjectWriteError::Failed {
                    code: ProjectWriteErrorCode::FilesystemFailure,
                })?;
            service.stage_mutations.ensure_capacity(project_root)?;
            let (project, mut publication) =
                crate::project::scene::adoption::sync_linked_scene_atomic(
                    crate::project::scene::adoption::LinkedSceneSyncRequest {
                        project_root,
                        source: &source,
                        inspection: &inspection,
                        scene_id,
                        base_manifest: validated.raw(),
                    },
                )
                .map_err(|_| ProjectWriteError::Failed {
                    code: ProjectWriteErrorCode::FilesystemFailure,
                })?;
            if let Err(error) = service.stage_mutations.submit_for_project(
                project_root,
                super::ProjectStageMutation::RefreshSceneDefinition {
                    project_id,
                    scene_id,
                },
            ) {
                let _ = publication.rollback();
                return Err(error);
            }
            publication
                .finalize()
                .map_err(|_| ProjectWriteError::Failed {
                    code: ProjectWriteErrorCode::FilesystemFailure,
                })?;
            let summary = super::inspection::project_summary(&project.manifest, project_root)?;
            let _ = service.cache_warm.enqueue_affected(
                project_root,
                crate::project::cache::ProjectCacheTarget::Scene {
                    id: scene_id.to_string(),
                },
            );
            Ok(ProjectSceneAdoptionResponse {
                project: summary,
                scene_id,
                placement_id: None,
                operation_id: operation_id.clone(),
                generation,
                progress: ProjectImportProgress {
                    operation_id: operation_id.clone(),
                    generation,
                    phase: ProjectImportPhase::Completed,
                },
            })
        })();
    service.progress.publish(ProjectImportProgress {
        operation_id,
        generation,
        phase: if result.is_ok() {
            ProjectImportPhase::Completed
        } else {
            ProjectImportPhase::Failed
        },
    });
    result
}

fn adopt_scene_inner(
    service: &mut ProjectApplicationService,
    project_id: usd_project::ProjectId,
    target: ProjectWriteTarget,
    source: &Path,
    inspection: &CompositionInspection,
    name: String,
    operation_id: String,
    generation: u64,
    placement: PlacementSpec,
    linked_source: Option<&Path>,
) -> Result<ProjectSceneAdoptionResponse, ProjectWriteError> {
    let placement = placement
        .resolve()
        .map_err(|_| ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::InvalidPlacement,
        })?;
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

    let (parent_scene_id, set_as_root) = match target {
        ProjectWriteTarget::Project(target_project_id) if target_project_id == project_id => {
            match validated.raw().root {
                ProjectRoot::Scene(scene_id) => (Some(scene_id), false),
                ProjectRoot::Empty | ProjectRoot::Model(_) => {
                    return Err(ProjectWriteError::Invalid {
                        code: ProjectWriteErrorCode::InvalidRootForComposition,
                    });
                }
            }
        }
        ProjectWriteTarget::Project(_) | ProjectWriteTarget::Model(_) => {
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
            (Some(scene_id), false)
        }
    };

    service.progress.publish(ProjectImportProgress {
        operation_id: operation_id.clone(),
        generation,
        phase: ProjectImportPhase::Validating,
    });
    let project_root = entry.repository_locator();
    let graph = super::scene::scene_graph(project_root, &validated).map_err(|_| {
        ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::FilesystemFailure,
        }
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
        .unwrap_or_else(Vec::<SceneMember>::new);
    if parent_scene_id.is_some() {
        service.stage_mutations.ensure_capacity(project_root)?;
    }

    service.progress.publish(ProjectImportProgress {
        operation_id: operation_id.clone(),
        generation,
        phase: ProjectImportPhase::Publishing,
    });
    let adopted = crate::project::scene::adoption::adopt_scene_atomic(
        crate::project::scene::adoption::SceneAdoptionRequest {
            project_root,
            source,
            inspection,
            name: &name,
            base_manifest: validated.raw(),
            graph: &graph,
            parent_scene_id,
            parent_members: &parent_members,
            target_scene_id: None,
            set_as_root,
            placement,
            linked_source,
        },
    )
    .map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::FilesystemFailure,
    })?;
    let project = super::inspection::project_summary(&adopted.manifest, project_root)?;
    if let Some(parent_scene_id) = parent_scene_id {
        service.stage_mutations.submit_for_project(
            project_root,
            super::ProjectStageMutation::AdoptScene {
                project_id,
                scene_id: adopted.scene_id,
                parent_scene_id: Some(parent_scene_id),
                placement: adopted.member.clone(),
            },
        )?;
    }
    let _ = service.cache_warm.enqueue_affected(
        project_root,
        crate::project::cache::ProjectCacheTarget::Scene {
            id: adopted.scene_id.to_string(),
        },
    );

    Ok(ProjectSceneAdoptionResponse {
        project,
        scene_id: adopted.scene_id,
        placement_id: adopted.member.map(|member| member.id),
        operation_id: operation_id.clone(),
        generation,
        progress: ProjectImportProgress {
            operation_id,
            generation,
            phase: ProjectImportPhase::Completed,
        },
    })
}

#[cfg(test)]
#[path = "scene_adoption_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "scene_adoption_sync_tests.rs"]
mod sync_tests;
