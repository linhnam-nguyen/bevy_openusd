use std::fs;

use bevy::prelude::World;
use tempfile::tempdir;
use usd_bevy::{LiveStage, PrimEntities};
use viewport_protocol::RuntimeProfile;

use super::{Spawned, StageInfo, activate_stage_with_cache_context};
use crate::project::cache::{ProjectCacheStore, ProjectCacheTarget};
use crate::project::cache_hydration::{
    ActiveProjectCacheContext, default_project_cache_config_hash,
};

#[test]
fn corrupt_cache_falls_back_to_a_successfully_opened_canonical_stage() {
    let project = tempdir().expect("temporary Project repository");
    usd_git::Repository::init(project.path()).expect("initialize Git repository");
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
