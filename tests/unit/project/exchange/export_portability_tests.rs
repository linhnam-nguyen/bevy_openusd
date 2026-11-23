use std::{fs, fs::File, io::Read};

use super::ProjectApplicationService;
use project_protocol::{
    LocalSelectionToken, PlacementSpec, ProjectExportSceneRequest, ProjectWriteTarget,
};
use tempfile::tempdir;

#[test]
fn registered_project_exports_loose_nested_payload_layers() {
    let dir = tempdir().unwrap();
    let mut creator = ProjectApplicationService::open(dir.path().join("creator.json")).unwrap();
    let project = creator.create_project(dir.path(), "Loose Export").unwrap();
    let root = dir.path().join("Loose Export");
    let scene = creator
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "LooseScene",
        )
        .unwrap();
    let scene_path = crate::project::scene::authoring::scene_path(&root, scene.scene_id);
    let parts = scene_path.with_file_name("parts.usda");
    let nested = parts.with_file_name("nested.usda");
    fs::write(&parts, "#usda 1.0\ndef Xform \"Part\" {\n def Xform \"Nested\" (payload = @./nested.usda@</Nested>) {}\n}\n").unwrap();
    fs::write(
        &nested,
        "#usda 1.0\ndef Xform \"Nested\" {\n def Cube \"Child\" { double size = 0.7 }\n}\n",
    )
    .unwrap();
    let stage = openusd::usd::Stage::open(scene_path.to_str().unwrap()).unwrap();
    let owner = stage.define_prim("/SceneRoot/Loose").unwrap();
    owner
        .set_metadata(
            "payload",
            openusd::vt::Value::PayloadListOp(openusd::sdf::PayloadListOp::prepended([
                openusd::sdf::Payload {
                    asset_path: "./parts.usda".to_owned(),
                    prim_path: openusd::sdf::path("/Part").unwrap(),
                    ..Default::default()
                },
            ])),
        )
        .unwrap();
    stage
        .root_layer()
        .export(scene_path.to_str().unwrap())
        .unwrap();
    drop(stage);
    creator
        .commit(project_protocol::ProjectCommitRequest {
            project_id: project.id,
            target: project_protocol::ProjectCommitTarget::Project,
            message: "Add nested payload".to_owned(),
        })
        .unwrap();

    let mut service = ProjectApplicationService::open(dir.path().join("importer.json")).unwrap();
    let inspection = service.inspect_project(&root).unwrap();
    let imported = service.import_project(&root, &inspection).unwrap();
    let destination = dir.path().join("loose.usdz");
    let request = ProjectExportSceneRequest {
        project_id: imported.id,
        scene_id: scene.scene_id,
        destination: LocalSelectionToken::new("loose-export"),
    };
    service.export_scene(request.clone(), &destination).unwrap();
    let exported = usd_bevy::LiveStage::open_authoritative(&destination).unwrap();
    let child = exported.prim("/SceneRoot/Loose/Nested/Child").unwrap();
    assert_eq!(child.attribute("size").get::<f64>().unwrap(), Some(0.7));

    let previous = fs::read(&destination).unwrap();
    let outside = dir.path().join("outside.usda");
    fs::rename(&nested, &outside).unwrap();
    assert!(service.export_scene(request.clone(), &destination).is_err());
    assert_eq!(fs::read(&destination).unwrap(), previous);

    fs::write(
        &parts,
        format!(
            "#usda 1.0\ndef Xform \"Part\" {{\n def Xform \"Nested\" (payload = @{}@</Nested>) {{}}\n}}\n",
            outside.to_str().unwrap().replace('\\', "/")
        ),
    )
    .unwrap();
    assert!(service.export_scene(request, &destination).is_err());
    assert_eq!(fs::read(&destination).unwrap(), previous);
}

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
