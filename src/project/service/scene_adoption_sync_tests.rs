use std::fs;

use openusd::{sdf::Value, usd::PrimPredicate};
use project_protocol::{PlacementSpec, ProjectWriteTarget};
use tempfile::tempdir;

use super::ProjectApplicationService;
use crate::project::{
    cache::{SceneCacheDescriptorV3, SceneCacheStore},
    cache_hydration::default_project_cache_config_hash,
    scene::inspection::inspect_composition,
    service::ProjectStageMutationQueue,
    storage::ProjectStorageLayout,
};

#[test]
fn syncing_linked_child_refreshes_an_active_project_root() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let source = directory.path().join("external.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\") {}\n",
    )
    .unwrap();
    let stage_mutations = ProjectStageMutationQueue::default();
    let mut service = ProjectApplicationService::open_with_project_state(
        directory.path().join("workspace.json"),
        Default::default(),
        stage_mutations.clone(),
    )
    .unwrap();
    let project = service.create_project(&parent, "Root Refresh").unwrap();
    let project_root = parent.join("Root Refresh");
    let root_scene_id = match project.root {
        usd_project::ProjectRoot::Scene(scene_id) => scene_id,
        _ => panic!("new Project must have a protected root Scene"),
    };
    let inspection = inspect_composition(&source).unwrap();
    let linked = service
        .link_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            &source,
            &inspection,
            "External Assembly".to_owned(),
            "link-operation".to_owned(),
            1,
            PlacementSpec::Default,
        )
        .unwrap();
    let cache = crate::project::cache::SceneCacheStore::new(&project_root);
    let root_generation_after_placement = cache.load_descriptor(root_scene_id).unwrap().unwrap().generation;
    let child_generation_before_sync = cache.load_descriptor(linked.scene_id).unwrap().unwrap().generation;
    let root_path = crate::project::scene::authoring::scene_path(&project_root, root_scene_id);
    let mut live_root = usd_bevy::LiveStage::new(
        openusd::usd::Stage::open(root_path.to_string_lossy().as_ref()).unwrap(),
    );
    assert_eq!(
        stage_mutations
            .apply_for_active_scene(
                &mut live_root,
                &project_root,
                project.id,
                Some(root_scene_id)
            )
            .unwrap(),
        1,
        "initial child placement should be applied to the active root"
    );
    let _ = live_root.drain_change_batch();

    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\") { string version = \"updated\" }\n",
    )
    .unwrap();
    assert_eq!(
        crate::project::link::status(&project_root, linked.scene_id).unwrap(),
        crate::project::link::LinkedSourceStatus::OutOfSync
    );
    service
        .sync_linked_scene(project.id, linked.scene_id, "sync-operation".to_owned(), 2)
        .unwrap();
    assert_eq!(cache.load_descriptor(root_scene_id).unwrap().unwrap().generation, root_generation_after_placement);
    assert!(cache.load_descriptor(linked.scene_id).unwrap().unwrap().generation > child_generation_before_sync);

    assert_eq!(
        stage_mutations
            .apply_for_active_scene(
                &mut live_root,
                &project_root,
                project.id,
                Some(root_scene_id)
            )
            .unwrap(),
        1,
        "syncing a composed child should refresh the active root"
    );
    let refresh = live_root.drain_change_batch().expect("root refresh batch");
    assert!(refresh.has_resync());
    let mut found_updated_source = false;
    let mut paths = Vec::new();
    live_root
        .stage
        .traverse(PrimPredicate::DEFAULT, |path| {
            paths.push(path.clone());
        })
        .unwrap();
    for path in paths {
        if let Ok(Some(Value::String(version))) = live_root
            .stage
            .prim(path)
            .attribute("version")
            .get::<Value>()
        {
            found_updated_source |= version == "updated";
        }
    }
    assert!(
        found_updated_source,
        "active root should expose the refreshed child"
    );
}

#[test]
fn syncing_linked_scene_cleans_overflowing_cache_before_success() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let source = directory.path().join("external.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\") {}\n",
    )
    .unwrap();
    let mut service = ProjectApplicationService::open(directory.path().join("workspace.json"))
        .unwrap();
    let project = service.create_project(&parent, "Overflow Sync").unwrap();
    let project_root = parent.join("Overflow Sync");
    let inspection = inspect_composition(&source).unwrap();
    let linked = service
        .link_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            &source,
            &inspection,
            "External Assembly".to_owned(),
            "overflow-sync-link".to_owned(),
            1,
            PlacementSpec::Default,
        )
        .unwrap();
    assert!(service.wait_for_cache_idle(&project_root));

    let cache = SceneCacheStore::new(&project_root);
    cache
        .publish_descriptor(&SceneCacheDescriptorV3::invalidated(
            linked.scene_id,
            u64::MAX,
            default_project_cache_config_hash(),
        ))
        .unwrap();
    assert!(cache.load_descriptor(linked.scene_id).unwrap().is_some());

    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\") { string version = \"updated\" }\n",
    )
    .unwrap();
    service.cache_warm.shutdown_without_waiting();

    let response = service
        .sync_linked_scene(project.id, linked.scene_id, "overflow-sync".to_owned(), 2)
        .unwrap();
    assert_eq!(response.scene_id, linked.scene_id);
    assert!(cache.load_descriptor(linked.scene_id).unwrap().is_none());
    assert!(!ProjectStorageLayout::new(&project_root)
        .scene_cache_dir(linked.scene_id)
        .exists());
}
