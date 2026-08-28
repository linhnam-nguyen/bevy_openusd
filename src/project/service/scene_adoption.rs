//! Application service for authoritative composed Scene adoption.

use std::path::Path;

use project_protocol::{
    ProjectImportPhase, ProjectImportProgress, ProjectSceneAdoptionResponse, ProjectWriteError,
    ProjectWriteErrorCode, ProjectWriteTarget,
};
use usd_project::{CompositionInspection, ProjectRoot, SceneMember};

use super::ProjectApplicationService;

pub(super) fn adopt_scene(
    service: &mut ProjectApplicationService,
    project_id: usd_project::ProjectId,
    target: ProjectWriteTarget,
    source: &Path,
    inspection: &CompositionInspection,
    operation_id: String,
    generation: u64,
) -> Result<ProjectSceneAdoptionResponse, ProjectWriteError> {
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

    let adopted = crate::project::scene::adoption::adopt_scene_atomic(
        crate::project::scene::adoption::SceneAdoptionRequest {
            project_root,
            source,
            inspection,
            base_manifest: validated.raw(),
            graph: &graph,
            parent_scene_id,
            parent_members: &parent_members,
            target_scene_id: None,
            set_as_root,
        },
    )
    .map_err(|_| ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::FilesystemFailure,
    })?;
    let project = super::inspection::project_summary(&adopted.manifest, project_root)?;
    service
        .stage_mutations
        .submit(super::ProjectStageMutation::AdoptScene {
            project_id,
            scene_id: adopted.scene_id,
            parent_scene_id,
            placement_id: adopted.member.as_ref().map(|member| member.id),
        })?;

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
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use usd_project::ProjectRoot;

    use super::*;
    use crate::project::{
        scene::inspection::inspect_composition, service::ProjectApplicationService,
    };

    #[test]
    fn empty_project_adoption_returns_no_placement_and_publishes_scene() {
        let directory = tempdir().unwrap();
        let parent = directory.path().join("projects");
        fs::create_dir(&parent).unwrap();
        let mut service =
            ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
        let summary = service.create_project(&parent, "Project").unwrap();
        let project_root = parent.join("Project");
        let source = project_root.join("source.usda");
        fs::write(
            &source,
            "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\") {}\n",
        )
        .unwrap();
        let inspection = inspect_composition(&source).unwrap();
        let adopted = service
            .adopt_scene(
                summary.id,
                ProjectWriteTarget::Project(summary.id),
                &source,
                &inspection,
                "operation-1".to_owned(),
                1,
            )
            .unwrap();
        assert!(adopted.placement_id.is_none());
        assert_eq!(adopted.operation_id, "operation-1");
        assert!(matches!(adopted.project.root, ProjectRoot::Scene(_)));
        assert_eq!(adopted.progress.operation_id, "operation-1");
        assert_eq!(adopted.progress.generation, 1);
        assert_eq!(adopted.progress.phase, ProjectImportPhase::Completed);
        assert!(project_root.join(".usdhub/scenes").is_dir());
    }

    #[test]
    fn nested_adoption_adds_one_identity_preserving_parent_placement() {
        let directory = tempdir().unwrap();
        let parent = directory.path().join("projects");
        fs::create_dir(&parent).unwrap();
        let mut service =
            ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
        let summary = service.create_project(&parent, "Project").unwrap();
        let project_root = parent.join("Project");
        let source = project_root.join("source.usda");
        fs::write(
            &source,
            "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\") {}\n",
        )
        .unwrap();
        let inspection = inspect_composition(&source).unwrap();
        let first = service
            .adopt_scene(
                summary.id,
                ProjectWriteTarget::Project(summary.id),
                &source,
                &inspection,
                "operation-root".to_owned(),
                1,
            )
            .unwrap();
        let nested = service
            .adopt_scene(
                summary.id,
                ProjectWriteTarget::Scene(first.scene_id),
                &source,
                &inspection,
                "operation-nested".to_owned(),
                2,
            )
            .unwrap();

        assert_ne!(first.scene_id, nested.scene_id);
        let placement_id = nested.placement_id.expect("nested adoption placement");
        let members = crate::project::scene::authoring::read_scene_members(
            &crate::project::scene::authoring::scene_path(&project_root, first.scene_id),
            first.scene_id,
        )
        .unwrap();
        assert!(members.iter().any(|member| {
            member.id == placement_id
                && member.target == usd_project::SceneMemberTarget::Scene(nested.scene_id)
        }));
    }
}
