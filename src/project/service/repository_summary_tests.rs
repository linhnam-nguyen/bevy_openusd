use std::{path::Path, process::Command};

use project_protocol::{ProjectReadCommand, ProjectReadRequest, ProjectReadResponse};
use tempfile::tempdir;
use usd_project::{ProjectId, ProjectManifestV1, ProjectRoot};

use super::{
    ManifestStore, ProjectApplicationService, ProjectPublicationCoordinator,
    ProjectStageMutationQueue, WorkspaceRegistry,
};

#[test]
fn repository_summary_projects_git_state_without_backend_handles() {
    let directory = tempdir().unwrap();
    let registry_path = directory.path().join("workspace.json");
    let project_id = ProjectId::new_v4();
    let repository = directory.path().join("repository");
    std::fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init", "-b", "main"]);
    run_git(&repository, &["config", "user.name", "USDHub Test"]);
    run_git(
        &repository,
        &["config", "user.email", "test@usdhub.invalid"],
    );
    let manifest = ProjectManifestV1::new(
        project_id,
        "Project",
        ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(&repository, &manifest).unwrap();
    std::fs::write(repository.join("notes.txt"), b"clean").unwrap();
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-m", "initial Project"]);

    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(project_id, &repository, None).unwrap();
    let service = ProjectApplicationService {
        registry,
        publication_coordinator: ProjectPublicationCoordinator::default(),
        stage_mutations: ProjectStageMutationQueue::default(),
    };

    let read = || {
        service.execute(ProjectReadCommand::new(
            ProjectReadRequest::GetProjectRepositorySummary(project_id),
        ))
    };
    let reply = read();
    let ProjectReadResponse::RepositorySummary { repository, .. } = reply.result.unwrap() else {
        panic!("repository request must return RepositorySummary");
    };
    assert_eq!(repository.active_branch.as_deref(), Some("main"));
    assert_eq!(
        repository
            .branches
            .iter()
            .map(|branch| branch.name.as_str())
            .collect::<Vec<_>>(),
        vec!["main"]
    );
    assert!(repository.head.is_some());
    assert!(!repository.dirty);

    std::fs::write(
        service
            .registry
            .get(project_id)
            .expect("registered Project")
            .repository_locator()
            .join("notes.txt"),
        b"dirty",
    )
    .unwrap();
    let reply = read();
    let ProjectReadResponse::RepositorySummary { repository, .. } = reply.result.unwrap() else {
        panic!("repository request must return RepositorySummary");
    };
    assert!(repository.dirty);
    let encoded = serde_json::to_string(&repository).unwrap();
    assert!(!encoded.contains("gix"));
    assert!(!encoded.contains("notes.txt"));
}

#[test]
fn repository_summary_preserves_an_unborn_symbolic_branch() {
    let directory = tempdir().unwrap();
    let registry_path = directory.path().join("workspace.json");
    let project_id = ProjectId::new_v4();
    let repository = directory.path().join("repository");
    std::fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init", "-b", "main"]);
    let manifest = ProjectManifestV1::new(
        project_id,
        "Unborn Project",
        ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(&repository, &manifest).unwrap();

    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(project_id, &repository, None).unwrap();
    let service = ProjectApplicationService {
        registry,
        publication_coordinator: ProjectPublicationCoordinator::default(),
        stage_mutations: ProjectStageMutationQueue::default(),
    };
    let reply = service.execute(ProjectReadCommand::new(
        ProjectReadRequest::GetProjectRepositorySummary(project_id),
    ));
    let ProjectReadResponse::RepositorySummary { repository, .. } = reply.result.unwrap() else {
        panic!("repository request must return RepositorySummary");
    };

    assert_eq!(repository.active_branch.as_deref(), Some("main"));
    assert!(repository.branches.is_empty());
    assert!(repository.head.is_none());
}

fn run_git(directory: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run git command");
    assert!(
        output.status.success(),
        "git command failed: {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}
