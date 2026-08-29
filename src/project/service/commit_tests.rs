use project_protocol::{ProjectCommitRequest, ProjectCommitTarget, ProjectWriteTarget};
use tempfile::tempdir;
use usd_git::GitRepository;

use super::ProjectApplicationService;

#[test]
fn project_commit_publishes_revision_and_clears_authoritative_worktree() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    std::fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service
        .create_project(&parent, "Committed Project")
        .unwrap();
    let project_root = parent.join("Committed Project");

    let response = service
        .commit(ProjectCommitRequest {
            project_id: project.id,
            target: ProjectCommitTarget::Project,
            message: "Create Project baseline".to_owned(),
        })
        .unwrap();

    assert_eq!(response.project.id, project.id);
    assert_eq!(response.revision.id.len(), 40);
    assert_eq!(response.repository.head.unwrap().id, response.revision.id);
    assert!(!response.repository.dirty);
    let repository = usd_git::Repository::open(&project_root).unwrap();
    assert!(!repository.working_tree_status().unwrap().dirty);
    assert_eq!(
        repository
            .read_commit(&usd_git::RevisionId::new(response.revision.id))
            .unwrap()
            .message,
        "Create Project baseline\n"
    );
}

#[test]
fn scene_commit_targets_scene_scope_and_returns_new_revision() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    std::fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service
        .create_project(&parent, "Scene Commit Project")
        .unwrap();
    service
        .commit(ProjectCommitRequest {
            project_id: project.id,
            target: ProjectCommitTarget::Project,
            message: "Create Project baseline".to_owned(),
        })
        .unwrap();
    let scene = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Architecture",
        )
        .unwrap();

    let response = service
        .commit(ProjectCommitRequest {
            project_id: project.id,
            target: ProjectCommitTarget::Scene(scene.scene_id),
            message: "Add Architecture scene".to_owned(),
        })
        .unwrap();

    assert_eq!(response.project.id, project.id);
    assert_eq!(response.repository.head.unwrap().id, response.revision.id);
    assert!(!response.repository.dirty);
    assert_eq!(response.revision.id.len(), 40);
}

#[test]
fn commit_rejects_empty_message_without_changing_repository() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    std::fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Message Project").unwrap();

    let result = service.commit(ProjectCommitRequest {
        project_id: project.id,
        target: ProjectCommitTarget::Project,
        message: "   ".to_owned(),
    });

    assert!(matches!(
        result,
        Err(project_protocol::ProjectWriteError::Invalid {
            code: project_protocol::ProjectWriteErrorCode::CommitMessageInvalid
        })
    ));
    let repository = usd_git::Repository::open(parent.join("Message Project")).unwrap();
    assert!(repository.head().unwrap().is_none());
}
