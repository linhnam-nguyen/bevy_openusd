//! Application service for authoritative Model publication.

use std::path::Path;

use project_protocol::{
    ProjectImportPhase, ProjectImportProgress, ProjectModelWriteResponse, ProjectWriteError,
    ProjectWriteErrorCode, ProjectWriteTarget,
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
) -> Result<ProjectModelWriteResponse, ProjectWriteError> {
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
                ProjectRoot::Empty => (None, true),
                ProjectRoot::Scene(scene_id) => (Some(scene_id), false),
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

    let placement = parent_scene_id.map(|parent_scene_id| ModelPlacement {
        parent_scene_id,
        parent_members: &parent_members,
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
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use usd_project::{ProjectRoot, SceneMemberTarget};

    use super::*;

    fn model_source(directory: &std::path::Path) -> std::path::PathBuf {
        let source = directory.join("asset.usda");
        fs::write(
            &source,
            "#usda 1.0\n(\n defaultPrim = \"Asset\"\n)\ndef Xform \"Asset\" (kind = \"component\") {}\n",
        )
        .unwrap();
        source
    }

    fn service_with_project(
        directory: &std::path::Path,
    ) -> (ProjectApplicationService, usd_project::ProjectSummary) {
        let parent = directory.join("projects");
        fs::create_dir(&parent).unwrap();
        let mut service =
            ProjectApplicationService::open(directory.join("workspace.json")).unwrap();
        let project = service.create_project(&parent, "Project").unwrap();
        (service, project)
    }

    #[test]
    fn empty_project_model_import_becomes_root_without_a_placement() {
        let directory = tempdir().unwrap();
        let (mut service, project) = service_with_project(directory.path());
        let source = model_source(directory.path());
        let queue = ProjectModelPreparationQueue::default();
        let preparation = queue.prepare("model-root".to_owned(), 1, source.clone());
        assert!(preparation.inspection.is_ok());

        let response = publish_model(
            &mut service,
            &queue,
            project.id,
            ProjectWriteTarget::Project(project.id),
            &source,
            "model-root".to_owned(),
            1,
        )
        .unwrap();

        assert!(response.placement_id.is_none());
        assert_eq!(response.project.root, ProjectRoot::Model(response.model_id));
        assert_eq!(response.project.counts.models, 1);
    }

    #[test]
    fn scene_target_model_import_adds_one_model_placement() {
        let directory = tempdir().unwrap();
        let (mut service, project) = service_with_project(directory.path());
        let scene = service
            .create_scene(project.id, ProjectWriteTarget::Project(project.id), "Scene")
            .unwrap();
        let source = model_source(directory.path());
        let queue = ProjectModelPreparationQueue::default();
        queue.prepare("model-scene".to_owned(), 2, source.clone());

        let response = publish_model(
            &mut service,
            &queue,
            project.id,
            ProjectWriteTarget::Scene(scene.scene_id),
            &source,
            "model-scene".to_owned(),
            2,
        )
        .unwrap();

        let placement_id = response.placement_id.expect("Scene target placement");
        let members = crate::project::scene::authoring::read_scene_members(
            &crate::project::scene::authoring::scene_path(
                &directory.path().join("projects/Project"),
                scene.scene_id,
            ),
            scene.scene_id,
        )
        .unwrap();
        assert!(members.iter().any(|member| {
            member.id == placement_id
                && member.target == SceneMemberTarget::Model(response.model_id)
        }));
    }

    #[test]
    fn model_root_rejects_a_second_model_import_before_consuming_preparation() {
        let directory = tempdir().unwrap();
        let (mut service, project) = service_with_project(directory.path());
        let source = model_source(directory.path());
        let queue = ProjectModelPreparationQueue::default();
        queue.prepare("first".to_owned(), 1, source.clone());
        let first = publish_model(
            &mut service,
            &queue,
            project.id,
            ProjectWriteTarget::Project(project.id),
            &source,
            "first".to_owned(),
            1,
        )
        .unwrap();
        assert_eq!(first.project.root, ProjectRoot::Model(first.model_id));

        queue.prepare("second".to_owned(), 2, source.clone());
        let error = publish_model(
            &mut service,
            &queue,
            project.id,
            ProjectWriteTarget::Project(project.id),
            &source,
            "second".to_owned(),
            2,
        )
        .unwrap_err();
        assert_eq!(
            error,
            ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::InvalidRootForComposition
            }
        );
    }
}
