use project_protocol::{ProjectCommitRequest, ProjectCommitTarget, ProjectWriteTarget};
use std::fs;
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
fn scene_commit_uses_fixed_point_ancestors_and_preserves_unrelated_dirty_manifest() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Closure Project").unwrap();
    let project_root = parent.join("Closure Project");
    let first = service
        .create_scene(project.id, ProjectWriteTarget::Project(project.id), "First")
        .unwrap();
    let target = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Scene(first.scene_id),
            "Target",
        )
        .unwrap();
    let unrelated = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Unrelated",
        )
        .unwrap();
    service
        .commit(ProjectCommitRequest {
            project_id: project.id,
            target: ProjectCommitTarget::Project,
            message: "baseline closure".to_owned(),
        })
        .unwrap();

    let root_scene_id =
        match crate::project::catalog::manifest_store::ManifestStore::read_validated(&project_root)
            .unwrap()
            .raw()
            .root
        {
            usd_project::ProjectRoot::Scene(scene_id) => scene_id,
            usd_project::ProjectRoot::Empty | usd_project::ProjectRoot::Model(_) => {
                panic!("project has a protected root scene")
            }
        };
    add_scene_marker(&project_root, root_scene_id, "ChangedRoot");
    add_scene_marker(&project_root, first.scene_id, "ChangedFirst");
    add_scene_marker(&project_root, target.scene_id, "ChangedTarget");
    let mut manifest =
        crate::project::catalog::manifest_store::ManifestStore::read_validated(&project_root)
            .unwrap()
            .raw()
            .clone();
    manifest
        .scenes
        .iter_mut()
        .find(|scene| scene.id == unrelated.scene_id)
        .unwrap()
        .display_name = "Unrelated Renamed".to_owned();
    crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
        &project_root,
        &manifest,
    )
    .unwrap();

    let response = service
        .commit(ProjectCommitRequest {
            project_id: project.id,
            target: ProjectCommitTarget::Scene(target.scene_id),
            message: "target closure".to_owned(),
        })
        .unwrap();
    let repository = usd_git::Repository::open(&project_root).unwrap();
    let materialized = tempdir().unwrap();
    repository
        .materialize_revision(
            &usd_git::RevisionId::new(response.revision.id.clone()),
            materialized.path(),
        )
        .unwrap();

    let committed_manifest =
        crate::project::catalog::manifest_store::ManifestStore::read_validated(&project_root)
            .unwrap();
    for (scene_id, marker) in [
        (root_scene_id, "ChangedRoot"),
        (first.scene_id, "ChangedFirst"),
        (target.scene_id, "ChangedTarget"),
    ] {
        let path = if scene_id == root_scene_id {
            materialized.path().join("Closure Project.usda")
        } else {
            let scene = committed_manifest.scene(scene_id).unwrap();
            materialized
                .path()
                .join("scenes")
                .join(format!("{}.usda", scene.storage_key))
        };
        let content = fs::read_to_string(path).unwrap();
        assert!(
            content.contains(marker),
            "closure omitted {scene_id}: {marker}"
        );
    }
    let staged_manifest: usd_project::ProjectManifestV1 =
        serde_json::from_slice(&fs::read(materialized.path().join("project.json")).unwrap())
            .unwrap();
    assert_eq!(
        staged_manifest
            .scenes
            .iter()
            .find(|scene| scene.id == unrelated.scene_id)
            .unwrap()
            .display_name,
        "Unrelated"
    );
    assert!(repository.working_tree_status().unwrap().dirty);
    assert!(response.repository.dirty);
}

fn add_scene_marker(project_root: &std::path::Path, scene_id: usd_project::SceneId, name: &str) {
    let path = crate::project::scene::authoring::scene_path(project_root, scene_id);
    let stage = openusd::usd::Stage::open(path.to_string_lossy().as_ref()).unwrap();
    usd_bevy::authoring::define_prim(&stage, &format!("/SceneRoot/{name}"), "Xform").unwrap();
    let temporary = path.with_file_name(format!(".{}.{}.tmp.usda", scene_id, name));
    stage
        .root_layer()
        .export(temporary.to_string_lossy().as_ref())
        .unwrap();
    fs::rename(temporary, path).unwrap();
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
