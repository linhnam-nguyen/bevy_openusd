use std::fs::File;

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
