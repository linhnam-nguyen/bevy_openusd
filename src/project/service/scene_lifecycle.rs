//! Application service for conservative Scene lifecycle mutations.

use project_protocol::{
    ProjectDeleteSceneRequest, ProjectSceneLifecycleResponse, ProjectWriteError,
    ProjectWriteErrorCode,
};

use super::ProjectApplicationService;

impl ProjectApplicationService {
    pub fn delete_scene(
        &mut self,
        request: ProjectDeleteSceneRequest,
    ) -> Result<ProjectSceneLifecycleResponse, ProjectWriteError> {
        super::deletion::delete_scene(self, request)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use project_protocol::ProjectWriteTarget;
    use tempfile::tempdir;
    use usd_project::ProjectRoot;

    use super::*;
    use crate::project::scene::authoring::{read_scene_members, scene_path};

    #[test]
    fn protected_root_scene_cannot_be_deleted() {
        let directory = tempdir().unwrap();
        let parent = directory.path().join("projects");
        fs::create_dir(&parent).unwrap();
        let mut service =
            ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
        let summary = service.create_project(&parent, "Project").unwrap();
        let project_root = parent.join("Project");
        let root_scene_id = match summary.root {
            ProjectRoot::Scene(scene_id) => scene_id,
            _ => panic!("new Project must have a protected Root Scene"),
        };

        assert_eq!(
            service.delete_scene(ProjectDeleteSceneRequest {
                project_id: summary.id,
                scene_id: root_scene_id,
            }),
            Err(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::ProtectedRootScene
            })
        );
        assert!(scene_path(&project_root, root_scene_id).is_file());
    }

    #[test]
    fn referenced_scene_deletion_removes_all_incoming_placements() {
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

        service
            .delete_scene(ProjectDeleteSceneRequest {
                project_id: summary.id,
                scene_id: child.scene_id,
            })
            .unwrap();
        assert!(!scene_path(&root, child.scene_id).exists());
        assert!(
            read_scene_members(&scene_path(&root, root_scene.scene_id), root_scene.scene_id)
                .unwrap()
                .is_empty()
        );
    }
}
