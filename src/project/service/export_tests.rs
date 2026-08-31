use std::{fs::File, io::Read};

use super::ProjectApplicationService;
use project_protocol::{
    LocalSelectionToken, PlacementSpec, ProjectExportSceneRequest, ProjectWriteTarget,
};
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
fn exported_scene_round_trips_through_inspection_and_default_or_matrix_import() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    std::fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let source_project = service.create_project(&parent, "Roundtrip Source").unwrap();
    let source_scene_id = match source_project.root {
        usd_project::ProjectRoot::Scene(scene_id) => scene_id,
        root => panic!("expected protected Scene root, found {root:?}"),
    };
    let exports = directory.path().join("exports");
    std::fs::create_dir(&exports).unwrap();
    let destination = exports.join("roundtrip.usdz");

    service
        .export_scene(
            ProjectExportSceneRequest {
                project_id: source_project.id,
                scene_id: source_scene_id,
                destination: LocalSelectionToken::new("roundtrip-export"),
            },
            &destination,
        )
        .unwrap();
    let inspection = crate::project::scene::inspection::inspect_composition(&destination).unwrap();
    assert!(matches!(
        inspection.classification,
        usd_project::CompositionClassification::NativeUsdHubScene
            | usd_project::CompositionClassification::SceneLike
    ));

    let target_project = service.create_project(&parent, "Roundtrip Target").unwrap();
    let imported = service
        .adopt_scene(
            target_project.id,
            ProjectWriteTarget::Project(target_project.id),
            &destination,
            &inspection,
            "Imported Default".to_owned(),
            "roundtrip-default".to_owned(),
            1,
            PlacementSpec::Default,
        )
        .unwrap();
    assert!(imported.placement_id.is_some());

    let placed = service
        .adopt_scene(
            target_project.id,
            ProjectWriteTarget::Scene(imported.scene_id),
            &destination,
            &inspection,
            "Imported Corrected".to_owned(),
            "roundtrip-matrix".to_owned(),
            2,
            PlacementSpec::Matrix("1 0 0 0\n0 1 0 0\n0 0 1 0\n7 8 9 1".to_owned()),
        )
        .unwrap();
    let placement_id = placed.placement_id.expect("matrix import placement");
    let target_root = parent.join("Roundtrip Target");
    let members = crate::project::scene::authoring::read_scene_members(
        &crate::project::scene::authoring::scene_path(&target_root, imported.scene_id),
        imported.scene_id,
    )
    .unwrap();
    assert!(members.iter().any(|member| {
        member.id == placement_id
            && member.target == usd_project::SceneMemberTarget::Scene(placed.scene_id)
            && member.transform
                == usd_project::ScenePlacementTransform::from_translation([7.0, 8.0, 9.0])
    }));
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
