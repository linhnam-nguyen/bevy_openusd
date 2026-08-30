use std::fs;

use super::*;
use crate::project::{
    scene::inspection::inspect_composition,
    service::{ProjectApplicationService, ProjectImportProgressStore},
};
use tempfile::tempdir;

#[test]
fn project_level_adoption_places_scene_under_the_protected_root() {
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
            "Assembly".to_owned(),
            "operation-1".to_owned(),
            1,
            project_protocol::PlacementSpec::Default,
        )
        .unwrap();
    assert!(adopted.placement_id.is_some());
    assert_eq!(adopted.operation_id, "operation-1");
    assert_eq!(adopted.project.root, summary.root);
    assert_eq!(adopted.progress.operation_id, "operation-1");
    assert_eq!(adopted.progress.generation, 1);
    assert_eq!(adopted.progress.phase, ProjectImportPhase::Completed);
    assert!(project_root.join(".usdhub/scenes").is_dir());
}

#[test]
fn linked_scene_keeps_snapshot_when_external_source_disappears() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let source = directory.path().join("external.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\") {}\n",
    )
    .unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let summary = service.create_project(&parent, "Project").unwrap();
    let inspection = inspect_composition(&source).unwrap();

    let linked = service
        .link_scene(
            summary.id,
            project_protocol::ProjectWriteTarget::Project(summary.id),
            &source,
            &inspection,
            "External Assembly".to_owned(),
            "link-operation".to_owned(),
            1,
            PlacementSpec::Default,
        )
        .unwrap();

    assert_eq!(
        crate::project::link::status(&parent.join("Project"), linked.scene_id).unwrap(),
        crate::project::link::LinkedSourceStatus::InSync
    );
    assert!(
        crate::project::scene::authoring::scene_path(&parent.join("Project"), linked.scene_id)
            .is_file()
    );

    fs::remove_file(&source).unwrap();
    assert_eq!(
        crate::project::link::status(&parent.join("Project"), linked.scene_id).unwrap(),
        crate::project::link::LinkedSourceStatus::SourceUnavailable
    );
    assert!(
        crate::project::scene::authoring::scene_path(&parent.join("Project"), linked.scene_id)
            .is_file()
    );
}

#[test]
fn syncing_linked_scene_replaces_closure_without_changing_scene_identity() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let source = directory.path().join("external.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\") {}\n",
    )
    .unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let summary = service.create_project(&parent, "Project").unwrap();
    let inspection = inspect_composition(&source).unwrap();
    let linked = service
        .link_scene(
            summary.id,
            ProjectWriteTarget::Project(summary.id),
            &source,
            &inspection,
            "External Assembly".to_owned(),
            "link-operation".to_owned(),
            1,
            PlacementSpec::Default,
        )
        .unwrap();
    let project_root = parent.join("Project");
    let before = fs::read(
        project_root
            .join(".usdhub/imports/scenes")
            .join(linked.scene_id.to_string())
            .join("external.usda"),
    )
    .unwrap();

    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\") { string version = \"updated\" }\n",
    )
    .unwrap();
    let refreshed_inspection = inspect_composition(&source).unwrap();
    let synced = service
        .sync_linked_scene(
            summary.id,
            linked.scene_id,
            &source,
            &refreshed_inspection,
            "External Assembly".to_owned(),
            "sync-operation".to_owned(),
            2,
        )
        .unwrap();

    assert_eq!(synced.scene_id, linked.scene_id);
    assert_ne!(
        fs::read(
            project_root
                .join(".usdhub/imports/scenes")
                .join(linked.scene_id.to_string())
                .join("external.usda"),
        )
        .unwrap(),
        before
    );
    assert_eq!(
        crate::project::link::status(&project_root, linked.scene_id).unwrap(),
        crate::project::link::LinkedSourceStatus::InSync
    );
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
            "Assembly".to_owned(),
            "operation-root".to_owned(),
            1,
            project_protocol::PlacementSpec::Default,
        )
        .unwrap();
    let nested = service
        .adopt_scene(
            summary.id,
            ProjectWriteTarget::Scene(first.scene_id),
            &source,
            &inspection,
            "Nested Assembly".to_owned(),
            "operation-nested".to_owned(),
            2,
            project_protocol::PlacementSpec::Matrix(
                "1 0 0 0\n0 1 0 0\n0 0 1 0\n3 4 5 1".to_owned(),
            ),
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
            && member.transform
                == usd_project::ScenePlacementTransform::from_translation([3.0, 4.0, 5.0])
    }));
}

#[test]
fn adoption_publishes_backend_owned_terminal_progress() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let progress = ProjectImportProgressStore::default();
    let mut service = ProjectApplicationService::open_with_project_state_and_progress(
        directory.path().join("workspace.json"),
        Default::default(),
        Default::default(),
        progress.clone(),
    )
    .unwrap();
    let project = service.create_project(&parent, "Project").unwrap();
    let source = directory.path().join("assembly.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\") {}\n",
    )
    .unwrap();
    let inspection = inspect_composition(&source).unwrap();

    service
        .adopt_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            &source,
            &inspection,
            "Assembly".to_owned(),
            "adoption-progress".to_owned(),
            5,
            project_protocol::PlacementSpec::Default,
        )
        .unwrap();

    assert_eq!(
        progress.latest("adoption-progress", 5).unwrap().phase,
        ProjectImportPhase::Completed
    );
}
