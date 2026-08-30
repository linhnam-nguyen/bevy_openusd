use std::{fs::File, io::Read};

use super::ProjectApplicationService;
use project_protocol::{LocalSelectionToken, ProjectExportSceneRequest, ProjectWriteTarget};
use tempfile::tempdir;

#[test]
fn scene_export_contains_the_selected_scene_dependency_closure() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    std::fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Export Project").unwrap();
    let scene = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Architecture",
        )
        .unwrap();
    let child = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Scene(scene.scene_id),
            "Interior",
        )
        .unwrap();
    let output_directory = directory.path().join("exports");
    std::fs::create_dir(&output_directory).unwrap();
    let destination = output_directory.join("architecture.usdz");

    let response = service
        .export_scene(
            ProjectExportSceneRequest {
                project_id: project.id,
                scene_id: scene.scene_id,
                destination: LocalSelectionToken::new("save-token"),
            },
            &destination,
        )
        .unwrap();

    assert_eq!(response.project_id, project.id);
    assert_eq!(response.scene_id, scene.scene_id);
    assert_eq!(response.file_name, "architecture.usdz");
    let file = File::open(&destination).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    assert_eq!(archive.by_index(0).unwrap().name(), "scene.usda");
    assert!(
        archive
            .by_name(&format!("scenes/{}.usda", scene.scene_id))
            .is_ok()
    );
    assert!(
        archive
            .by_name(&format!("scenes/{}.usda", child.scene_id))
            .is_ok()
    );
    drop(archive);

    let stage = openusd::usd::Stage::open(destination.to_string_lossy().as_ref()).unwrap();
    stage
        .traverse(openusd::usd::PrimPredicate::DEFAULT, |_| {})
        .unwrap();
}

#[test]
fn scene_export_rejects_non_usdz_destination_without_creating_a_file() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    std::fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Invalid Export").unwrap();
    let scene = service
        .create_scene(project.id, ProjectWriteTarget::Project(project.id), "Scene")
        .unwrap();
    let destination = directory.path().join("scene.usda");

    let result = service.export_scene(
        ProjectExportSceneRequest {
            project_id: project.id,
            scene_id: scene.scene_id,
            destination: LocalSelectionToken::new("save-token"),
        },
        &destination,
    );

    assert!(matches!(
        result,
        Err(project_protocol::ProjectWriteError::Invalid {
            code: project_protocol::ProjectWriteErrorCode::ExportDestinationInvalid
        })
    ));
    assert!(!destination.exists());
}

#[test]
fn live_stage_export_uses_the_exact_current_revision() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    std::fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Live Export").unwrap();
    let scene = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Active Scene",
        )
        .unwrap();
    let project_root = parent.join("Live Export");
    let scene_path = crate::project::scene::authoring::scene_path(&project_root, scene.scene_id);
    let live = usd_bevy::LiveStage::new(
        openusd::usd::Stage::open(scene_path.to_string_lossy().as_ref()).unwrap(),
    );
    usd_bevy::authoring::define_prim(&live.stage, "/SceneRoot/LiveOnly", "Xform").unwrap();
    let revision = live
        .drain_change_batch()
        .expect("live edit should advance the revision")
        .revision;
    let manifest =
        crate::project::catalog::manifest_store::ManifestStore::read_validated(&project_root)
            .unwrap();
    let output_directory = directory.path().join("exports");
    std::fs::create_dir(&output_directory).unwrap();
    let stale_destination = output_directory.join("stale.usdz");
    assert!(
        super::export::write_live_stage_archive(
            &project_root,
            &manifest,
            scene.scene_id,
            &live,
            usd_bevy::LiveRevision::default(),
            &stale_destination,
        )
        .is_err()
    );
    assert!(!stale_destination.exists());

    let destination = output_directory.join("live.usdz");
    super::export::write_live_stage_archive(
        &project_root,
        &manifest,
        scene.scene_id,
        &live,
        revision,
        &destination,
    )
    .unwrap();
    let file = File::open(destination).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let mut root = String::new();
    archive
        .by_name("scene.usda")
        .unwrap()
        .read_to_string(&mut root)
        .unwrap();
    assert!(root.contains("LiveOnly"));
}
