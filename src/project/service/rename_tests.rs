use std::fs;

use openusd::{sdf, sdf::Value, usd::Stage};
use project_protocol::{PlacementSpec, ProjectWriteTarget};
use tempfile::tempdir;
use usd_project::{
    ProjectRoot, SceneMember, SceneMemberId, SceneMemberTarget, ScenePlacementTransform,
};

use super::*;
use crate::project::{
    catalog::manifest_store::ManifestStore,
    scene::authoring::{read_scene_members, scene_path},
    service::{ProjectApplicationService, ProjectModelPreparationQueue, ProjectStageMutationQueue},
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

#[test]
fn failed_rename_restores_all_canonical_files_and_stage_outbox() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let queue = ProjectStageMutationQueue::default();
    let mut service = ProjectApplicationService::open_with_stage_mutation_queue(
        directory.path().join("workspace.json"),
        queue.clone(),
    )
    .unwrap();
    let project = service.create_project(&parent, "Project").unwrap();
    let project_root = parent.join("Project");
    let scene = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Architecture",
        )
        .unwrap();
    let scene_path = scene_path(&project_root, scene.scene_id);
    let manifest_path = crate::project::catalog::manifest_store::manifest_path(&project_root);
    let scene_before = fs::read(&scene_path).unwrap();
    let manifest_before = fs::read(&manifest_path).unwrap();
    let pending_before = stage_outbox_count(&project_root);
    queue.fail_before_batch_index(0);

    assert!(
        service
            .rename(
                project.id,
                ProjectWriteTarget::Scene(scene.scene_id),
                "Architecture Revised",
            )
            .is_err()
    );

    assert_eq!(fs::read(scene_path).unwrap(), scene_before);
    assert_eq!(fs::read(manifest_path).unwrap(), manifest_before);
    assert_eq!(stage_outbox_count(&project_root), pending_before);
}

#[test]
fn model_rename_updates_every_self_describing_placement_without_aliases() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Project").unwrap();
    let project_root = parent.join("Project");
    let first_scene = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "First Scene",
        )
        .unwrap();
    let second_scene = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Second Scene",
        )
        .unwrap();
    let source = directory.path().join("chair.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Asset\"\n)\ndef Xform \"Asset\" (kind = \"component\") {}\n",
    )
    .unwrap();
    let preparation = ProjectModelPreparationQueue::default();
    preparation.prepare("model".to_owned(), 1, source.clone());
    let published = service
        .publish_model(
            &preparation,
            project.id,
            ProjectWriteTarget::Scene(first_scene.scene_id),
            &source,
            "model".to_owned(),
            1,
            PlacementSpec::Matrix("1 0 0 0\n0 1 0 0\n0 0 1 0\n10 0 0 1".to_owned()),
        )
        .unwrap();
    let first_placement_id = published.placement_id.unwrap();
    let second_placement = SceneMember {
        id: SceneMemberId::new_v4(),
        target: SceneMemberTarget::Model(published.model_id),
        name: Some("chair".to_owned()),
        transform: ScenePlacementTransform::from_translation([0.0, 20.0, 0.0]),
    };
    crate::project::scene::authoring::author_scene_atomic_with_members(
        &project_root,
        second_scene.scene_id,
        std::slice::from_ref(&second_placement),
    )
    .unwrap();

    let model_wrapper =
        crate::project::model_wrapper::model_wrapper_path(&project_root, published.model_id);
    for (scene_id, placement_id, expected_translation) in [
        (first_scene.scene_id, first_placement_id, [10.0, 0.0, 0.0]),
        (second_scene.scene_id, second_placement.id, [0.0, 20.0, 0.0]),
    ] {
        let path = scene_path(&project_root, scene_id);
        let stage = Stage::open(path.to_string_lossy().as_ref()).unwrap();
        let member =
            stage.prim(crate::project::scene::authoring::scene_member_path(placement_id).as_str());
        let Some(Value::Dictionary(data)) = member.custom_data().unwrap() else {
            panic!("model placement must have customData");
        };
        assert_eq!(
            data.get("usdhub:memberId").and_then(Value::as_str),
            Some(placement_id.to_string().as_str())
        );
        assert_eq!(
            data.get("usdhub:targetKind").and_then(Value::as_str),
            Some("model")
        );
        assert_eq!(
            data.get("usdhub:targetId").and_then(Value::as_str),
            Some(published.model_id.to_string().as_str())
        );
        assert!(!data.contains_key("usdhub:name"));
        assert_eq!(
            stage
                .root_layer()
                .prim(
                    &sdf::path(crate::project::scene::authoring::scene_member_path(
                        placement_id
                    ),)
                    .unwrap()
                )
                .unwrap()
                .field("ui:displayName")
                .unwrap(),
            Some(Value::String("chair".to_owned()))
        );
        let transform = member
            .attribute("xformOp:transform")
            .get::<Value>()
            .unwrap()
            .unwrap();
        let Value::Matrix4d(transform) = transform else {
            panic!("model placement must own a matrix transform");
        };
        assert_eq!(transform.0[12..15], expected_translation);
        let root_layer = stage.root_layer();
        let spec = root_layer
            .prim(
                &sdf::path(crate::project::scene::authoring::scene_member_path(
                    placement_id,
                ))
                .unwrap(),
            )
            .unwrap();
        let Some(Value::ReferenceListOp(references)) = spec.field("references").unwrap() else {
            panic!("model placement must reference the canonical wrapper");
        };
        let reference = references.iter().next().unwrap();
        assert_eq!(reference.prim_path.as_str(), "/ModelRoot");
        assert_eq!(
            reference.asset_path,
            crate::project::storage::authored_relative_project_asset_path(
                &project_root,
                &path,
                &model_wrapper,
            )
            .unwrap()
        );
    }

    service
        .rename(
            project.id,
            ProjectWriteTarget::Model(published.model_id),
            "Chair Renamed",
        )
        .unwrap();
    let manifest = ManifestStore::read_validated(&project_root).unwrap();
    assert_eq!(
        manifest.model(published.model_id).unwrap().display_name,
        "Chair Renamed"
    );
    let wrapper = Stage::open(model_wrapper.to_string_lossy().as_ref()).unwrap();
    assert_eq!(
        wrapper
            .root_layer()
            .prim(&sdf::path("/ModelRoot").unwrap())
            .unwrap()
            .field("ui:displayName")
            .unwrap(),
        Some(Value::String("Chair Renamed".to_owned()))
    );
    for scene_id in [first_scene.scene_id, second_scene.scene_id] {
        let members = read_scene_members(&scene_path(&project_root, scene_id), scene_id).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(
            members[0].target,
            SceneMemberTarget::Model(published.model_id)
        );
        assert_eq!(members[0].name.as_deref(), Some("Chair Renamed"));
        let stage = Stage::open(
            scene_path(&project_root, scene_id)
                .to_string_lossy()
                .as_ref(),
        )
        .unwrap();
        let Some(Value::Dictionary(data)) = stage
            .prim(crate::project::scene::authoring::scene_member_path(members[0].id).as_str())
            .custom_data()
            .unwrap()
        else {
            panic!("renamed model placement must retain customData");
        };
        assert!(!data.contains_key("usdhub:name"));
    }
}

#[test]
fn model_rename_failure_on_second_placement_restores_everything() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Project").unwrap();
    let project_root = parent.join("Project");
    let first_scene = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "First Scene",
        )
        .unwrap();
    let second_scene = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Second Scene",
        )
        .unwrap();
    let source = directory.path().join("chair.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Asset\"\n)\ndef Xform \"Asset\" (kind = \"component\") {}\n",
    )
    .unwrap();
    let preparation = ProjectModelPreparationQueue::default();
    preparation.prepare("model".to_owned(), 1, source.clone());
    let published = service
        .publish_model(
            &preparation,
            project.id,
            ProjectWriteTarget::Scene(first_scene.scene_id),
            &source,
            "model".to_owned(),
            1,
            PlacementSpec::Default,
        )
        .unwrap();
    let second_placement = SceneMember {
        id: SceneMemberId::new_v4(),
        target: SceneMemberTarget::Model(published.model_id),
        name: Some("chair".to_owned()),
        transform: Default::default(),
    };
    crate::project::scene::authoring::author_scene_atomic_with_members(
        &project_root,
        second_scene.scene_id,
        std::slice::from_ref(&second_placement),
    )
    .unwrap();

    let manifest_path = crate::project::catalog::manifest_store::manifest_path(&project_root);
    let wrapper_path =
        crate::project::model_wrapper::model_wrapper_path(&project_root, published.model_id);
    let paths = [
        manifest_path,
        wrapper_path,
        scene_path(&project_root, first_scene.scene_id),
        scene_path(&project_root, second_scene.scene_id),
    ];
    let before = paths
        .iter()
        .map(fs::read)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    set_test_failure_after_placement(1);
    let result = service.rename(
        project.id,
        ProjectWriteTarget::Model(published.model_id),
        "Chair Renamed",
    );
    clear_test_failure_after_placement();
    assert!(result.is_err());
    for (path, expected) in paths.iter().zip(before) {
        assert_eq!(fs::read(path).unwrap(), expected);
    }
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
