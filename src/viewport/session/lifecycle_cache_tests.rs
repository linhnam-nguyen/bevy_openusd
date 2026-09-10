use std::{fs, sync::{Arc, Mutex}, thread, time::Duration};

use bevy::prelude::World;
use openusd::usd::Stage;
use tempfile::tempdir;
use usd_bevy::{LiveStage, PrimEntities, ProjectionSeed};
use usd_model::HashDigest;
use viewport_protocol::{PrimNodeReadModel, RuntimeProfile, SceneAnchor};

use super::{
    Spawned, StageInfo, activate_stage_with_cache_context, poll_scene_cache_revalidation,
    activate_stage_with_cache_context_for_test,
};
use crate::project::cache_contract::{SceneCacheDescriptorV3, SceneCacheState};
use crate::project::cache::{ProjectCacheStore, ProjectCacheTarget};
use crate::project::cache_hydration::{
    ActiveProjectCacheContext, default_project_cache_config_hash,
};
use crate::project::catalog::manifest_store::ManifestStore;
use crate::viewport::api::CurrentHierarchyProjection;
use crate::viewport::session::{PendingSceneCacheRevalidation, SceneCachePresentation};

#[test]
fn corrupt_cache_falls_back_to_a_successfully_opened_canonical_stage() {
    let project = tempdir().expect("temporary Project repository");
    usd_git::Repository::init(project.path()).expect("initialize Git repository");
    let manifest = usd_project::ProjectManifestV1::new(
        usd_project::ProjectId::new_v4(),
        "Cache fallback fixture",
        usd_project::ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(project.path(), &manifest)
        .expect("write Project manifest");
    let stage_path = project.path().join("stage.usda");
    fs::write(&stage_path, "#usda 1.0\n\ndef Xform \"World\" {}\n").expect("write canonical stage");
    let context = ActiveProjectCacheContext::new(
        project.path().to_path_buf(),
        ProjectCacheTarget::ProjectRoot,
        RuntimeProfile::NativeMedium,
        default_project_cache_config_hash(),
    )
    .expect("create cache identity");
    let descriptor_path = ProjectCacheStore::new(project.path())
        .descriptor_path(&context.identity)
        .expect("resolve descriptor path");
    fs::create_dir_all(descriptor_path.parent().expect("descriptor directory"))
        .expect("create descriptor directory");
    fs::write(descriptor_path, b"corrupt descriptor").expect("write corrupt descriptor");

    let mut world = World::new();
    world.insert_resource(PrimEntities::default());
    world.insert_resource(Spawned::default());
    world.insert_resource(StageInfo::default());

    activate_stage_with_cache_context(&mut world, stage_path.clone(), Some(context))
        .expect("canonical source must remain openable after cache corruption");

    assert_eq!(
        world.resource::<StageInfo>().path,
        stage_path.to_string_lossy().into_owned()
    );
    assert!(world.get_non_send::<LiveStage>().is_some());
}

#[test]
fn changed_source_across_stage_open_cannot_consume_old_cache_seeds() {
    let project = tempdir().expect("temporary Project repository");
    usd_git::Repository::init(project.path()).expect("initialize Git repository");
    let project_id = usd_project::ProjectId::new_v4();
    let scene_id = usd_project::SceneId::new_v4();
    let manifest = usd_project::ProjectManifestV1::new(
        project_id,
        "Stage open identity race fixture",
        usd_project::ProjectRoot::Scene(scene_id),
        vec![usd_project::SceneManifestEntry {
            id: scene_id,
            storage_key: usd_project::StorageKey::new("scene").expect("Scene storage key"),
            display_name: "Stage open identity race fixture".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(project.path(), &manifest).expect("Project manifest");

    let scene_path = crate::project::scene::authoring::scene_path(project.path(), scene_id);
    fs::create_dir_all(scene_path.parent().expect("Scene directory"))
        .expect("create Scene directory");
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/stages/mesh_correctness.usda");
    let project_source = project.path().join("source-a.usda");
    fs::copy(&source, &project_source).expect("copy source into Project");
    let spatial = crate::project::spatial::inspect_source(&source).expect("inspect source A");
    crate::project::scene::adoption_authoring::author_scene_wrapper_to_path(
        &scene_path,
        project.path(),
        &scene_path,
        scene_id,
        &project_source,
        &project_source,
        &["/World".to_owned()],
        "Stage open identity race fixture",
        &spatial,
        false,
    )
    .expect("write Scene wrapper A");
    let source_a = fs::read(&scene_path).expect("read Scene wrapper A");

    let target = ProjectCacheTarget::Scene {
        id: scene_id.to_string(),
    };
    let queue = crate::project::cache_warmer::ProjectCacheWarmQueue::default();
    assert!(queue.enqueue(project.path(), target.clone()));
    assert!(queue.wait_for_project_idle(project.path(), Duration::from_secs(2)));
    assert_eq!(
        queue.prepare_for_activation(project.path(), target.clone()),
        crate::project::cache_warmer::ProjectCachePreparation::Ready
    );
    let context = ActiveProjectCacheContext::new(
        project.path().to_path_buf(),
        target,
        RuntimeProfile::NativeMedium,
        default_project_cache_config_hash(),
    )
    .expect("cache identity A");
    let mut source_b = source_a;
    source_b.extend_from_slice(b"\n# source B\n");

    let mut world = World::new();
    world.insert_resource(PrimEntities::default());
    world.init_resource::<ProjectionSeed>();
    world.insert_resource(Spawned::default());
    world.insert_resource(StageInfo::default());
    activate_stage_with_cache_context_for_test(
        &mut world,
        scene_path.clone(),
        Some(context),
        || {
            fs::write(&scene_path, source_b)
                .expect("mutate source between identity capture and Stage::open")
        },
    )
    .expect("changed canonical source remains openable");

    let seed = world.resource::<ProjectionSeed>();
    assert_eq!(seed.pending_meshes(), 0, "old mesh seeds must be discarded");
    assert_eq!(
        seed.pending_materials(),
        0,
        "old material seeds must be discarded"
    );
    assert!(world.get_non_send::<LiveStage>().is_some());
}

#[test]
fn poisoned_scene_revalidation_discards_cache_presentation_without_losing_live_stage() {
    let project = tempdir().expect("temporary Project repository");
    let scene_id = usd_project::SceneId::new_v4();
    let generation = 7;
    let result = Arc::new(Mutex::new(None));
    let poisoned = Arc::clone(&result);
    thread::spawn(move || {
        let _guard = poisoned.lock().expect("poison fixture lock");
        panic!("poison Scene revalidation result");
    })
    .join()
    .expect_err("poison fixture thread must fail");

    let stage_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/stages/mesh_correctness.usda");
    let stage = Stage::open(stage_path.to_string_lossy().as_ref()).expect("open canonical stage");
    let mut world = World::new();
    world.insert_non_send(LiveStage::new(stage));
    world.insert_resource(StageInfo {
        activation_generation: generation,
        ..StageInfo::default()
    });
    world.insert_resource(CurrentHierarchyProjection::from_prim_nodes(
        &[PrimNodeReadModel {
            anchor: SceneAnchor::active_session("/World"),
            parent: None,
            label: "World".to_owned(),
            display_name: Some("World".to_owned()),
            visible: true,
            has_children: false,
        }],
        generation,
    ));
    world.insert_resource(SceneCachePresentation {
        scene_id,
        generation,
        state: SceneCacheState::Partial,
        entries: Vec::new(),
    });
    world.insert_resource(PendingSceneCacheRevalidation {
        project_root: project.path().to_path_buf(),
        scene_id,
        activation_generation: generation,
        scene_generation: generation,
        expected_hash: None,
        config_hash: HashDigest::new([0; HashDigest::BYTE_LEN]),
        result,
    });

    poll_scene_cache_revalidation(&mut world);

    assert!(world.get_non_send::<LiveStage>().is_some());
    assert!(world
        .get_resource::<PendingSceneCacheRevalidation>()
        .is_none());
    assert!(world.get_resource::<SceneCachePresentation>().is_none());
    assert!(world
        .resource::<CurrentHierarchyProjection>()
        .snapshot()
        .nodes
        .is_empty());
}

#[test]
fn unequal_activation_and_scene_generations_revalidate_current_scene_cache() {
    let project = tempdir().expect("temporary Project repository");
    let scene_id = usd_project::SceneId::new_v4();
    let activation_generation = 41;
    let scene_generation = 7;
    let config_hash = HashDigest::new([3; HashDigest::BYTE_LEN]);
    let expected_hash = HashDigest::new([4; HashDigest::BYTE_LEN]);
    let actual_hash = HashDigest::new([5; HashDigest::BYTE_LEN]);
    let store = crate::project::cache::SceneCacheStore::new(project.path());
    store
        .publish_descriptor(&SceneCacheDescriptorV3::invalidated(
            scene_id,
            scene_generation,
            config_hash,
        ))
        .expect("publish Scene cache descriptor");

    let result = Arc::new(Mutex::new(Some(Ok(actual_hash))));
    let stage_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/stages/mesh_correctness.usda");
    let stage = Stage::open(stage_path.to_string_lossy().as_ref()).expect("open canonical stage");
    let mut world = World::new();
    world.insert_non_send(LiveStage::new(stage));
    world.insert_resource(StageInfo {
        activation_generation,
        ..StageInfo::default()
    });
    world.insert_resource(CurrentHierarchyProjection::from_prim_nodes(
        &[PrimNodeReadModel {
            anchor: SceneAnchor::active_session("/World"),
            parent: None,
            label: "World".to_owned(),
            display_name: Some("World".to_owned()),
            visible: true,
            has_children: false,
        }],
        scene_generation,
    ));
    world.insert_resource(SceneCachePresentation {
        scene_id,
        generation: scene_generation,
        state: SceneCacheState::Partial,
        entries: Vec::new(),
    });
    world.insert_resource(PendingSceneCacheRevalidation {
        project_root: project.path().to_path_buf(),
        scene_id,
        activation_generation,
        scene_generation,
        expected_hash: Some(expected_hash),
        config_hash,
        result: Arc::clone(&result),
    });

    poll_scene_cache_revalidation(&mut world);

    assert!(result.lock().expect("result slot remains usable").is_none());
    assert!(world.get_non_send::<LiveStage>().is_some());
    assert!(world
        .get_resource::<PendingSceneCacheRevalidation>()
        .is_none());
    assert!(world.get_resource::<SceneCachePresentation>().is_none());
    assert!(world
        .resource::<CurrentHierarchyProjection>()
        .snapshot()
        .nodes
        .is_empty());
    assert_eq!(
        store
            .load_descriptor(scene_id)
            .expect("load invalidated Scene descriptor")
            .expect("Scene descriptor remains present")
            .generation,
        scene_generation + 1
    );
}
