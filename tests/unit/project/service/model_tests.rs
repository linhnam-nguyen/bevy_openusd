use std::fs;

use openusd::usd::Stage;
use tempfile::tempdir;
use usd_project::SceneMemberTarget;

use super::*;
use crate::project::service::project_support::service_with_project;

fn model_source(directory: &std::path::Path) -> std::path::PathBuf {
    let source = directory.join("asset.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Asset\"\n)\ndef Xform \"Asset\" (kind = \"component\") {}\n",
    )
    .unwrap();
    source
}

#[test]
fn project_level_model_import_places_under_the_protected_root() {
    let directory = tempdir().unwrap();
    let (mut service, project) = service_with_project(directory.path());
    let source = model_source(directory.path());
    let queue = ProjectModelPreparationQueue::default();
    let preparation = queue.prepare("model-root".to_owned(), 1, source.clone());
    assert!(preparation.inspection.is_ok());

    let response = publish_model(
        &mut service,
        &queue,
        project.id,
        ProjectWriteTarget::Project(project.id),
        &source,
        "model-root".to_owned(),
        1,
        PlacementSpec::Default,
    )
    .unwrap();

    assert!(response.placement_id.is_some());
    assert_eq!(response.project.root, project.root);
    assert_eq!(response.project.counts.models, 1);
    assert!(
        directory
            .path()
            .join("projects/Project/.usdhub/cache/project-stage-mutations")
            .exists()
    );
}

#[test]
fn scene_target_model_import_adds_one_model_placement() {
    let directory = tempdir().unwrap();
    let (mut service, project) = service_with_project(directory.path());
    let scene = service
        .create_scene(project.id, ProjectWriteTarget::Project(project.id), "Scene")
        .unwrap();
    let source = model_source(directory.path());
    let queue = ProjectModelPreparationQueue::default();
    queue.prepare("model-scene".to_owned(), 2, source.clone());

    let response = publish_model(
        &mut service,
        &queue,
        project.id,
        ProjectWriteTarget::Scene(scene.scene_id),
        &source,
        "model-scene".to_owned(),
        2,
        PlacementSpec::Default,
    )
    .unwrap();

    let placement_id = response.placement_id.expect("Scene target placement");
    let members = crate::project::scene::authoring::read_scene_members(
        &crate::project::scene::authoring::scene_path(
            &directory.path().join("projects/Project"),
            scene.scene_id,
        ),
        scene.scene_id,
    )
    .unwrap();
    assert!(members.iter().any(|member| {
        member.id == placement_id
            && member.target == SceneMemberTarget::Model(response.model_id)
            && member.name.as_deref() == Some("asset")
    }));
    let parent_stage = Stage::open(
        &crate::project::scene::authoring::scene_path(
            &directory.path().join("projects/Project"),
            scene.scene_id,
        )
        .to_string_lossy(),
    )
    .unwrap();
    let authored = parent_stage.root_layer().export_to_string().unwrap();
    assert!(authored.contains("references"));
    assert!(authored.contains(&response.model_id.to_string()));
}

#[test]
fn project_level_model_imports_share_the_protected_root() {
    let directory = tempdir().unwrap();
    let (mut service, project) = service_with_project(directory.path());
    let source = model_source(directory.path());
    let queue = ProjectModelPreparationQueue::default();
    queue.prepare("first".to_owned(), 1, source.clone());
    let first = publish_model(
        &mut service,
        &queue,
        project.id,
        ProjectWriteTarget::Project(project.id),
        &source,
        "first".to_owned(),
        1,
        PlacementSpec::Default,
    )
    .unwrap();
    assert!(first.placement_id.is_some());
    assert_eq!(first.project.root, project.root);

    queue.prepare("second".to_owned(), 2, source.clone());
    let second = publish_model(
        &mut service,
        &queue,
        project.id,
        ProjectWriteTarget::Project(project.id),
        &source,
        "second".to_owned(),
        2,
        PlacementSpec::Default,
    )
    .unwrap();
    assert!(second.placement_id.is_some());
    assert_eq!(second.project.root, project.root);
    assert_eq!(second.project.counts.models, 2);
}
