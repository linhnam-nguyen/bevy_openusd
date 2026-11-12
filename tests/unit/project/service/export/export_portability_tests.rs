use std::{fs, fs::File, io::Read};

use super::ProjectApplicationService;
use project_protocol::{
    LocalSelectionToken, PlacementSpec, ProjectExportSceneRequest, ProjectWriteTarget,
};
use tempfile::tempdir;

#[test]
fn localized_non_root_scene_export_rewrites_project_relative_closure() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Localized Export").unwrap();
    let parent_scene = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Assembly",
        )
        .unwrap();
    let source_directory = directory.path().join("source-closure");
    fs::create_dir(&source_directory).unwrap();
    fs::write(
        source_directory.join("dependency.usda"),
        "#usda 1.0\n( defaultPrim = \"Asset\" )\ndef Xform \"Asset\" {}\n",
    )
    .unwrap();
    let source = source_directory.join("assembly.usda");
    fs::write(
        &source,
        "#usda 1.0\n( defaultPrim = \"Assembly\" )\ndef Xform \"Assembly\" (kind = \"assembly\" references = @./dependency.usda@</Asset>) {}\n",
    )
    .unwrap();
    let inspection = crate::project::scene::inspection::inspect_composition(&source).unwrap();
    let localized_scene = service
        .adopt_scene(
            project.id,
            ProjectWriteTarget::Scene(parent_scene.scene_id),
            &source,
            &inspection,
            "Localized Child".to_owned(),
            "localized-child".to_owned(),
            1,
            PlacementSpec::Default,
        )
        .unwrap();
    let destination = directory.path().join("localized.usdz");

    service
        .export_scene(
            ProjectExportSceneRequest {
                project_id: project.id,
                scene_id: localized_scene.scene_id,
                destination: LocalSelectionToken::new("localized-export"),
            },
            &destination,
        )
        .unwrap();

    let file = File::open(&destination).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let mut root = String::new();
    archive
        .by_name("scene.usda")
        .unwrap()
        .read_to_string(&mut root)
        .unwrap();
    let localized_source = format!(
        "@imports/scenes/{}/assembly.usda@",
        localized_scene.scene_id
    );
    assert!(root.contains(&localized_source), "{root}");
    assert!(!root.contains("@../imports/"), "{root}");
    drop(archive);

    let stage = openusd::usd::Stage::open(destination.to_string_lossy().as_ref()).unwrap();
    stage
        .traverse(openusd::usd::PrimPredicate::DEFAULT, |_| {})
        .unwrap();
    let exported_inspection =
        crate::project::scene::inspection::inspect_composition(&destination).unwrap();
    let target_project = service.create_project(&parent, "Localized Import").unwrap();
    let imported = service
        .adopt_scene(
            target_project.id,
            ProjectWriteTarget::Project(target_project.id),
            &destination,
            &exported_inspection,
            "Imported Localized Child".to_owned(),
            "imported-localized-child".to_owned(),
            1,
            PlacementSpec::Default,
        )
        .unwrap();
    assert!(imported.placement_id.is_some());
}
