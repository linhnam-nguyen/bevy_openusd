//! Application service for conservative Scene lifecycle mutations.

use std::fs;

use project_protocol::{
    ProjectDeleteSceneRequest, ProjectRemoveScenePlacementRequest, ProjectSceneLifecycleResponse,
    ProjectWriteError, ProjectWriteErrorCode,
};
use usd_project::{ProjectRoot, SceneMemberTarget};
use uuid::Uuid;

use super::ProjectApplicationService;

impl ProjectApplicationService {
    pub fn remove_scene_placement(
        &mut self,
        request: ProjectRemoveScenePlacementRequest,
    ) -> Result<ProjectSceneLifecycleResponse, ProjectWriteError> {
        let (entry, validated) = self
            .validated_project(request.project_id)
            .map_err(|error| ProjectWriteError::Failed {
                code: match error {
                    project_protocol::ProjectReadError::NotFound { .. } => {
                        ProjectWriteErrorCode::ProjectNotFound
                    }
                    _ => ProjectWriteErrorCode::ScenePlacementRemoveFailed,
                },
            })?;
        if validated.scene(request.parent_scene_id).is_none() {
            return Err(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::SceneNotFound,
            });
        }
        let project_root = entry.repository_locator();
        let parent_path =
            crate::project::scene::authoring::scene_path(project_root, request.parent_scene_id);
        let members = crate::project::scene::authoring::read_scene_members(
            &parent_path,
            request.parent_scene_id,
        )
        .map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::ScenePlacementRemoveFailed,
        })?;
        if !members
            .iter()
            .any(|member| member.id == request.placement_id)
        {
            return Err(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::ScenePlacementNotFound,
            });
        }
        self.stage_mutations.ensure_capacity(project_root)?;
        crate::project::scene::authoring::remove_scene_member_atomic(
            &parent_path,
            request.parent_scene_id,
            request.placement_id,
        )
        .map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::ScenePlacementRemoveFailed,
        })?;
        self.stage_mutations.submit_for_project(
            project_root,
            super::ProjectStageMutation::RemoveScenePlacement {
                project_id: request.project_id,
                parent_scene_id: request.parent_scene_id,
                placement_id: request.placement_id,
            },
        )?;
        Ok(ProjectSceneLifecycleResponse {
            project_id: request.project_id,
            scene_id: request.parent_scene_id,
            placement_id: Some(request.placement_id),
        })
    }

    pub fn delete_scene(
        &mut self,
        request: ProjectDeleteSceneRequest,
    ) -> Result<ProjectSceneLifecycleResponse, ProjectWriteError> {
        let (entry, validated) = self
            .validated_project(request.project_id)
            .map_err(|error| ProjectWriteError::Failed {
                code: match error {
                    project_protocol::ProjectReadError::NotFound { .. } => {
                        ProjectWriteErrorCode::ProjectNotFound
                    }
                    _ => ProjectWriteErrorCode::SceneDeleteFailed,
                },
            })?;
        if validated.scene(request.scene_id).is_none() {
            return Err(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::SceneNotFound,
            });
        }
        let project_root = entry.repository_locator();
        for scene in validated.scenes() {
            let path = crate::project::scene::authoring::scene_path(project_root, scene.id);
            let members = crate::project::scene::authoring::read_scene_members(&path, scene.id)
                .map_err(|_| ProjectWriteError::Failed {
                    code: ProjectWriteErrorCode::SceneDeleteFailed,
                })?;
            if members
                .iter()
                .any(|member| member.target == SceneMemberTarget::Scene(request.scene_id))
            {
                return Err(ProjectWriteError::Invalid {
                    code: ProjectWriteErrorCode::SceneInUse,
                });
            }
        }

        self.stage_mutations.ensure_capacity(project_root)?;
        let scene_path =
            crate::project::scene::authoring::scene_path(project_root, request.scene_id);
        let tombstone = scene_path.with_file_name(format!(
            ".{}.delete-{}.usda",
            request.scene_id,
            Uuid::new_v4()
        ));
        fs::rename(&scene_path, &tombstone).map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::SceneDeleteFailed,
        })?;

        let previous_manifest = validated.raw().clone();
        let mut next_manifest = previous_manifest.clone();
        next_manifest
            .scenes
            .retain(|scene| scene.id != request.scene_id);
        if next_manifest.root == ProjectRoot::Scene(request.scene_id) {
            next_manifest.root = ProjectRoot::Empty;
        }
        if crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
            project_root,
            &next_manifest,
        )
        .is_err()
        {
            let _ = fs::rename(&tombstone, &scene_path);
            return Err(ProjectWriteError::Failed {
                code: ProjectWriteErrorCode::SceneDeleteFailed,
            });
        }
        if let Err(error) = self.stage_mutations.submit_for_project(
            project_root,
            super::ProjectStageMutation::DeleteScene {
                project_id: request.project_id,
                scene_id: request.scene_id,
            },
        ) {
            let _ = crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
                project_root,
                &previous_manifest,
            );
            let _ = fs::rename(&tombstone, &scene_path);
            return Err(error);
        }
        fs::remove_file(&tombstone).map_err(|_| ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::SceneDeleteCleanupFailed,
        })?;

        Ok(ProjectSceneLifecycleResponse {
            project_id: request.project_id,
            scene_id: request.scene_id,
            placement_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use project_protocol::{
        ProjectReadCommand, ProjectReadRequest, ProjectReadResponse, ProjectWriteTarget,
    };
    use tempfile::tempdir;
    use usd_project::ProjectRoot;

    use super::*;
    use crate::project::scene::adoption_authoring;
    use crate::project::scene::authoring::{read_scene_members, scene_path};

    #[test]
    fn remove_scene_placement_preserves_scene_definition() {
        let directory = tempdir().unwrap();
        let parent = directory.path().join("projects");
        fs::create_dir(&parent).unwrap();
        let mut service =
            ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
        let summary = service.create_project(&parent, "Project").unwrap();
        let root = parent.join("Project");
        let root_scene = service
            .create_scene(summary.id, ProjectWriteTarget::Project(summary.id), "Root")
            .unwrap();
        let child = service
            .create_scene(
                summary.id,
                ProjectWriteTarget::Scene(root_scene.scene_id),
                "Child",
            )
            .unwrap();
        let members =
            read_scene_members(&scene_path(&root, root_scene.scene_id), root_scene.scene_id)
                .unwrap();
        let placement = members.first().unwrap().id;

        service
            .remove_scene_placement(ProjectRemoveScenePlacementRequest {
                project_id: summary.id,
                parent_scene_id: root_scene.scene_id,
                placement_id: placement,
            })
            .unwrap();

        assert!(scene_path(&root, root_scene.scene_id).is_file());
        assert!(scene_path(&root, child.scene_id).is_file());
        assert!(
            read_scene_members(&scene_path(&root, root_scene.scene_id), root_scene.scene_id)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn removing_one_of_repeated_scene_placements_preserves_the_other() {
        let directory = tempdir().unwrap();
        let parent = directory.path().join("projects");
        fs::create_dir(&parent).unwrap();
        let mut service =
            ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
        let summary = service.create_project(&parent, "Project").unwrap();
        let root = parent.join("Project");
        let root_scene = service
            .create_scene(summary.id, ProjectWriteTarget::Project(summary.id), "Root")
            .unwrap();
        let child = service
            .create_scene(
                summary.id,
                ProjectWriteTarget::Scene(root_scene.scene_id),
                "Child",
            )
            .unwrap();
        let root_path = scene_path(&root, root_scene.scene_id);
        let first = read_scene_members(&root_path, root_scene.scene_id)
            .unwrap()
            .into_iter()
            .next()
            .expect("child placement");
        let second = usd_project::SceneMember {
            id: usd_project::SceneMemberId::new_v4(),
            target: usd_project::SceneMemberTarget::Scene(child.scene_id),
            name: Some("Repeated child placement".to_owned()),
            transform: Default::default(),
        };
        let temporary_path = root.join(format!(".{}.repeat.tmp.usda", root_scene.scene_id));
        adoption_authoring::prepare_parent_layer(
            &root_path,
            &temporary_path,
            &root,
            root_scene.scene_id,
            &[first.clone(), second.clone()],
        )
        .unwrap();
        fs::rename(&temporary_path, &root_path).unwrap();

        service
            .remove_scene_placement(ProjectRemoveScenePlacementRequest {
                project_id: summary.id,
                parent_scene_id: root_scene.scene_id,
                placement_id: second.id,
            })
            .unwrap();

        assert_eq!(
            read_scene_members(&root_path, root_scene.scene_id).unwrap(),
            vec![first]
        );
    }

    #[test]
    fn delete_unreferenced_root_scene_makes_project_empty() {
        let directory = tempdir().unwrap();
        let parent = directory.path().join("projects");
        fs::create_dir(&parent).unwrap();
        let mut service =
            ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
        let summary = service.create_project(&parent, "Project").unwrap();
        let root = parent.join("Project");
        let scene = service
            .create_scene(summary.id, ProjectWriteTarget::Project(summary.id), "Root")
            .unwrap();

        service
            .delete_scene(ProjectDeleteSceneRequest {
                project_id: summary.id,
                scene_id: scene.scene_id,
            })
            .unwrap();

        assert!(!scene_path(&root, scene.scene_id).exists());
        let reopened =
            ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
        let response = reopened
            .execute(ProjectReadCommand::new(ProjectReadRequest::ListProjects))
            .result
            .unwrap();
        let ProjectReadResponse::Projects(items) = response else {
            panic!("Project list response expected");
        };
        let project_protocol::ProjectListItem::Available(summary) = &items[0] else {
            panic!("available Project expected");
        };
        assert_eq!(summary.root, ProjectRoot::Empty);
    }

    #[test]
    fn referenced_scene_is_rejected_without_deleting_definition() {
        let directory = tempdir().unwrap();
        let parent = directory.path().join("projects");
        fs::create_dir(&parent).unwrap();
        let mut service =
            ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
        let summary = service.create_project(&parent, "Project").unwrap();
        let root = parent.join("Project");
        let root_scene = service
            .create_scene(summary.id, ProjectWriteTarget::Project(summary.id), "Root")
            .unwrap();
        let child = service
            .create_scene(
                summary.id,
                ProjectWriteTarget::Scene(root_scene.scene_id),
                "Child",
            )
            .unwrap();

        assert!(matches!(
            service.delete_scene(ProjectDeleteSceneRequest {
                project_id: summary.id,
                scene_id: child.scene_id,
            }),
            Err(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::SceneInUse
            })
        ));
        assert!(scene_path(&root, child.scene_id).is_file());
    }
}
