use std::fs;

use bevy::prelude::World;
use tempfile::tempdir;
use usd_bevy::{LiveStage, PrimEntities, ProjectionSeed};
use viewport_protocol::RuntimeProfile;

use super::{
    Spawned, StageInfo, activate_stage_with_cache_context,
    activate_stage_with_cache_context_for_test,
};
use crate::project::cache::{ProjectCacheStore, ProjectCacheTarget};
use crate::project::cache_hydration::{
    ActiveProjectCacheContext, default_project_cache_config_hash,
};
use crate::project::catalog::manifest_store::ManifestStore;

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

    let scene_path = project
        .path()
        .join(".usdhub/scenes")
        .join(format!("{scene_id}.usda"));
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
        "World",
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
    activate_stage_with_cache_context_for_test(&mut world, scene_path, Some(context), || {
        fs::write(
            project
                .path()
                .join(".usdhub/scenes")
                .join(format!("{scene_id}.usda")),
            source_b,
        )
        .expect("mutate source between identity capture and Stage::open")
    })
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
