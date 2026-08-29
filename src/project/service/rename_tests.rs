use std::fs;

use project_protocol::ProjectWriteTarget;
use tempfile::tempdir;
use usd_project::{ProjectRoot, SceneMemberTarget};

use super::*;
use crate::project::{
    catalog::manifest_store::ManifestStore,
    scene::authoring::{read_scene_members, scene_path},
    service::ProjectApplicationService,
};

#[test]
fn scene_rename_updates_the_manifest_and_every_placement_mirror() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Project").unwrap();
    let project_root = parent.join("Project");
    let scene = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Architecture",
        )
        .unwrap();

    service
        .rename(
            project.id,
            ProjectWriteTarget::Scene(scene.scene_id),
            "Architecture Revised",
        )
        .unwrap();

    let manifest = ManifestStore::read_validated(&project_root).unwrap();
    assert_eq!(
        manifest.scene(scene.scene_id).unwrap().display_name,
        "Architecture Revised"
    );
    let root_id = match &manifest.raw().root {
        ProjectRoot::Scene(id) => *id,
        other => panic!("unexpected root {other:?}"),
    };
    let members = read_scene_members(&scene_path(&project_root, root_id), root_id).unwrap();
    assert!(members.iter().any(|member| {
        member.target == SceneMemberTarget::Scene(scene.scene_id)
            && member.name.as_deref() == Some("Architecture Revised")
    }));
}

#[test]
fn project_rename_updates_the_protected_root_manifest_name() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Project").unwrap();

    let renamed = service
        .rename(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Renamed Project",
        )
        .unwrap();

    assert_eq!(renamed.project.name, "Renamed Project");
    let manifest = ManifestStore::read_validated(&parent.join("Project")).unwrap();
    assert_eq!(manifest.raw().name, "Renamed Project");
    let ProjectRoot::Scene(root_id) = manifest.raw().root else {
        panic!("Project must retain its protected root Scene");
    };
    assert_eq!(
        manifest.scene(root_id).unwrap().display_name,
        "Renamed Project"
    );
}
