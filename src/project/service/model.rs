//! Application service for authoritative Model publication.

use std::path::Path;

use project_protocol::{
    PlacementSpec, ProjectImportPhase, ProjectImportProgress, ProjectModelWriteResponse,
    ProjectWriteError, ProjectWriteErrorCode, ProjectWriteTarget,
};
use usd_project::ProjectRoot;

use super::{ProjectApplicationService, ProjectModelPreparationQueue};
use crate::project::model_wrapper::{
    ModelPlacement, ModelWrapperRequest, publish_model_wrapper_atomic,
};

pub(super) fn publish_model(
    service: &mut ProjectApplicationService,
    preparation: &ProjectModelPreparationQueue,
    project_id: usd_project::ProjectId,
    target: ProjectWriteTarget,
    source: &Path,
    operation_id: String,
    generation: u64,
    placement_spec: PlacementSpec,
) -> Result<ProjectModelWriteResponse, ProjectWriteError> {
    service.progress.publish(ProjectImportProgress {
        operation_id: operation_id.clone(),
        generation,
        phase: ProjectImportPhase::Validating,
    });
    let result = publish_model_inner(
        service,
        preparation,
        project_id,
        target,
        source,
        operation_id.clone(),
        generation,
        placement_spec,
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

fn publish_model_inner(
    service: &mut ProjectApplicationService,
    preparation: &ProjectModelPreparationQueue,
    project_id: usd_project::ProjectId,
    target: ProjectWriteTarget,
    source: &Path,
    operation_id: String,
    generation: u64,
    placement_spec: PlacementSpec,
) -> Result<ProjectModelWriteResponse, ProjectWriteError> {
    let placement_transform = placement_spec
        .resolve()
        .map_err(|_| ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::InvalidPlacement,
        })?;
    let publisher = service.publication_coordinator.publisher(project_id);
    let _publication = publisher
        .lock()
        .expect("Project publication lock is not poisoned");

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
            (Some(scene_id), false)
        }
        ProjectWriteTarget::Model(_) => {
            return Err(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::InvalidRootForComposition,
            });
        }
    };

    let prepared =
        preparation
            .take_prepared(&operation_id, generation)
            .ok_or(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::SelectionUnavailable,
            })?;
    if prepared.source != source {
        return Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::SelectionUnavailable,
        });
    }

    let project_root = entry.repository_locator();
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

    let placement = parent_scene_id.map(|parent_scene_id| ModelPlacement {
        parent_scene_id,
        parent_members: &parent_members,
        transform: placement_transform,
    });
    service.progress.publish(ProjectImportProgress {
        operation_id: operation_id.clone(),
        generation,
        phase: ProjectImportPhase::Publishing,
    });
    let published = publish_model_wrapper_atomic(ModelWrapperRequest {
        project_root,
        base_manifest: validated.raw(),
        prepared: &prepared,
        set_as_root,
        placement,
    })
    .map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::FilesystemFailure,
    })?;
    let project = super::inspection::project_summary(&published.manifest, project_root)?;
    if let Some(parent_scene_id) = parent_scene_id {
        service.stage_mutations.submit_for_project(
            project_root,
            super::ProjectStageMutation::PublishModel {
                project_id,
                model_id: published.id,
                parent_scene_id: Some(parent_scene_id),
                placement: published.placement.clone(),
            },
        )?;
    }
    let mut cache_targets = vec![
        crate::project::cache::ProjectCacheTarget::Model {
            id: published.id.to_string(),
        },
        crate::project::cache::ProjectCacheTarget::ProjectRoot,
    ];
    if let Some(parent_scene_id) = parent_scene_id {
        cache_targets.push(crate::project::cache::ProjectCacheTarget::Scene {
            id: parent_scene_id.to_string(),
        });
    }
    let _ = service
        .cache_warm
        .enqueue_targets_for_mutation(project_root, cache_targets)
        .map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::FilesystemFailure,
        })?;

    Ok(ProjectModelWriteResponse {
        project,
        model_id: published.id,
        placement_id: published.placement.map(|member| member.id),
        operation_id: operation_id.clone(),
        generation,
        progress: ProjectImportProgress {
            operation_id: operation_id.clone(),
            generation,
            phase: ProjectImportPhase::Completed,
        },
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/project/service/model_tests.rs"]
mod tests;
