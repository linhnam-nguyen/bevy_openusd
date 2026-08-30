use std::fs;

use project_protocol::{
    PlacementSpec, ProjectDeleteModelRequest, ProjectDeleteSceneRequest, ProjectWriteTarget,
};
use tempfile::tempdir;
use usd_project::{SceneMember, SceneMemberId, SceneMemberTarget, ScenePlacementTransform};

use super::{ProjectApplicationService, ProjectModelPreparationQueue, ProjectStageMutationQueue};

fn service_with_project(
    directory: &std::path::Path,
) -> (ProjectApplicationService, usd_project::ProjectSummary) {
    let parent = directory.join("projects");
    fs::create_dir(&parent).unwrap();
    let mut service = ProjectApplicationService::open(directory.join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Project").unwrap();
    (service, project)
}

fn add_scene_placement(
    project_root: &std::path::Path,
    parent_scene_id: usd_project::SceneId,
    child_scene_id: usd_project::SceneId,
) {
    let parent_path = crate::project::scene::authoring::scene_path(project_root, parent_scene_id);
    let mut members =
        crate::project::scene::authoring::read_scene_members(&parent_path, parent_scene_id)
            .unwrap();
    members.push(SceneMember {
        id: SceneMemberId::new_v4(),
        target: SceneMemberTarget::Scene(child_scene_id),
        name: Some("Shared Child".to_owned()),
        transform: ScenePlacementTransform::IDENTITY,
    });
    crate::project::scene::authoring::replace_scene_members_atomic(
        &parent_path,
        project_root,
        parent_scene_id,
        &members,
    )
    .unwrap();
}

#[test]
fn deleting_scene_preserves_descendant_shared_by_another_parent() {
    let directory = tempdir().unwrap();
    let (mut service, project) = service_with_project(directory.path());
    let project_root = directory.path().join("projects/Project");
    let parent = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Parent",
        )
        .unwrap();
    let other_parent = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Other Parent",
        )
        .unwrap();
    let shared = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Scene(parent.scene_id),
            "Shared Child",
        )
        .unwrap();
    add_scene_placement(&project_root, other_parent.scene_id, shared.scene_id);

    service
        .delete_scene(ProjectDeleteSceneRequest {
            project_id: project.id,
            scene_id: parent.scene_id,
        })
        .unwrap();

    let manifest =
        crate::project::catalog::manifest_store::ManifestStore::read_validated(&project_root)
            .unwrap();
    assert!(
        !manifest
            .scenes()
            .iter()
            .any(|scene| scene.id == parent.scene_id)
    );
    assert!(
        manifest
            .scenes()
            .iter()
            .any(|scene| scene.id == shared.scene_id)
    );
    assert!(!crate::project::scene::authoring::scene_path(&project_root, parent.scene_id).exists());
    let other_members = crate::project::scene::authoring::read_scene_members(
        &crate::project::scene::authoring::scene_path(&project_root, other_parent.scene_id),
        other_parent.scene_id,
    )
    .unwrap();
    assert!(
        other_members
            .iter()
            .any(|member| { member.target == SceneMemberTarget::Scene(shared.scene_id) })
    );
}

#[test]
fn deleting_scene_removes_unique_descendant_closure() {
    let directory = tempdir().unwrap();
    let (mut service, project) = service_with_project(directory.path());
    let project_root = directory.path().join("projects/Project");
    let parent = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Parent",
        )
        .unwrap();
    let child = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Scene(parent.scene_id),
            "Child",
        )
        .unwrap();

    service
        .delete_scene(ProjectDeleteSceneRequest {
            project_id: project.id,
            scene_id: parent.scene_id,
        })
        .unwrap();

    let manifest =
        crate::project::catalog::manifest_store::ManifestStore::read_validated(&project_root)
            .unwrap();
    assert!(
        !manifest
            .scenes()
            .iter()
            .any(|scene| scene.id == parent.scene_id)
    );
    assert!(
        !manifest
            .scenes()
            .iter()
            .any(|scene| scene.id == child.scene_id)
    );
    assert!(!crate::project::scene::authoring::scene_path(&project_root, child.scene_id).exists());
}

#[test]
fn deleting_model_removes_all_placements_but_not_external_source() {
    let directory = tempdir().unwrap();
    let (mut service, project) = service_with_project(directory.path());
    let project_root = directory.path().join("projects/Project");
    let source = directory.path().join("external-model.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Asset\"\n)\ndef Xform \"Asset\" (kind = \"component\") {}\n",
    )
    .unwrap();
    let queue = ProjectModelPreparationQueue::default();
    let preparation = queue.prepare("model-delete".to_owned(), 1, source.clone());
    assert!(preparation.inspection.is_ok());
    let published = service
        .publish_model(
            &queue,
            project.id,
            ProjectWriteTarget::Project(project.id),
            &source,
            "model-delete".to_owned(),
            1,
            PlacementSpec::Default,
        )
        .unwrap();
    let model_directory =
        crate::project::model_wrapper::model_wrapper_path(&project_root, published.model_id)
            .parent()
            .unwrap()
            .to_owned();

    let response = service
        .delete_model(ProjectDeleteModelRequest {
            project_id: project.id,
            model_id: published.model_id,
        })
        .unwrap();

    assert_eq!(response.placement_ids.len(), 1);
    assert!(!model_directory.exists());
    assert!(source.is_file());
    let manifest =
        crate::project::catalog::manifest_store::ManifestStore::read_validated(&project_root)
            .unwrap();
    assert!(
        !manifest
            .models()
            .iter()
            .any(|model| model.id == published.model_id)
    );
}

#[test]
fn failed_delete_restores_manifest_files_and_stage_outbox() {
    let directory = tempfile::tempdir().unwrap();
    let parent_directory = directory.path().join("projects");
    fs::create_dir(&parent_directory).unwrap();
    let queue = ProjectStageMutationQueue::default();
    let mut service = ProjectApplicationService::open_with_stage_mutation_queue(
        directory.path().join("workspace.json"),
        queue.clone(),
    )
    .unwrap();
    let project = service
        .create_project(&parent_directory, "Delete Rollback")
        .unwrap();
    let project_root = parent_directory.join("Delete Rollback");
    let parent = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Parent",
        )
        .unwrap();
    let child = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Scene(parent.scene_id),
            "Child",
        )
        .unwrap();
    let parent_path = crate::project::scene::authoring::scene_path(&project_root, parent.scene_id);
    let child_path = crate::project::scene::authoring::scene_path(&project_root, child.scene_id);
    let manifest_path = crate::project::catalog::manifest_store::manifest_path(&project_root);
    let parent_before = fs::read(&parent_path).unwrap();
    let child_before = fs::read(&child_path).unwrap();
    let manifest_before = fs::read(&manifest_path).unwrap();
    let pending_before = stage_outbox_count(&project_root);
    queue.fail_before_batch_index(1);

    assert!(
        service
            .delete_scene(ProjectDeleteSceneRequest {
                project_id: project.id,
                scene_id: parent.scene_id,
            })
            .is_err()
    );

    assert_eq!(fs::read(parent_path).unwrap(), parent_before);
    assert_eq!(fs::read(child_path).unwrap(), child_before);
    assert_eq!(fs::read(manifest_path).unwrap(), manifest_before);
    assert_eq!(stage_outbox_count(&project_root), pending_before);
}

fn stage_outbox_count(project_root: &std::path::Path) -> usize {
    let path = project_root
        .join(".usdhub")
        .join("cache")
        .join("project-stage-mutations");
    path.read_dir()
        .map(|entries| entries.count())
        .unwrap_or_default()
}
