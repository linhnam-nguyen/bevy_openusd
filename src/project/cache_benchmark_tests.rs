use std::time::Instant;

use bevy::{asset::Assets, image::Image, mesh::Mesh, pbr::StandardMaterial, prelude::App};
use openusd::usd::Stage;
use tempfile::tempdir;
use usd_model::HashDigest;
use viewport_protocol::RuntimeProfile;

use super::*;
use crate::project::cache::{
    ProjectCacheDescriptor, ProjectCacheIdentity, ProjectCacheState, ProjectCacheStore,
    ProjectCacheTarget,
};

fn digest(value: u8) -> HashDigest {
    HashDigest::new([value; HashDigest::BYTE_LEN])
}

#[test]
fn headless_cache_benchmark_proves_cold_persistent_and_hot_paths() -> Result<()> {
    let project = tempdir()?;
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/stages/mesh_correctness.usda");
    let identity = ProjectCacheIdentity {
        target: ProjectCacheTarget::ProjectRoot,
        target_content_hash: digest(8),
        profile: RuntimeProfile::NativeMedium,
        config_hash: default_project_cache_config_hash(),
    };

    let cold_start = Instant::now();
    let stage = Stage::open(&source.to_string_lossy())?;
    let cold_live = usd_bevy::LiveStage::new(stage.clone());
    let mut cold_app = headless_cache_app();
    let mut cold_map = usd_bevy::PrimEntities::default();
    usd_bevy::project_stage(cold_app.world_mut(), &cold_live, &mut cold_map);
    let cold_ms = cold_start.elapsed().as_secs_f64() * 1_000.0;
    let cold_projection = cold_app
        .world()
        .resource::<usd_bevy::route::cache::ProjectionCache>()
        .stats();
    let cold_material = cold_app
        .world()
        .resource::<usd_bevy::route::material::UsdMaterialCache>()
        .stats();
    assert!(cold_projection.misses > 0, "cold source mesh conversions");
    assert!(cold_material.misses > 0, "cold source material conversions");

    let persistent_build_start = Instant::now();
    let runtime = crate::project::cache_warm_runtime::build_runtime_cache(
        project.path(),
        &source,
        &identity,
    )?;
    let persistent_build_ms = persistent_build_start.elapsed().as_secs_f64() * 1_000.0;
    ProjectCacheStore::new(project.path()).publish(&ProjectCacheDescriptor::new(
        identity.clone(),
        ProjectCacheState::Ready,
        Some(runtime),
    )?)?;

    let persistent_start = Instant::now();
    let context = ActiveProjectCacheContext {
        project_root: project.path().to_path_buf(),
        identity,
    };
    let mut persistent_app = headless_cache_app();
    assert!(hydrate_project_cache(persistent_app.world_mut(), &context)?);
    let persistent_seed_meshes = persistent_app
        .world()
        .resource::<ProjectionSeed>()
        .pending_meshes();
    let persistent_seed_materials = persistent_app
        .world()
        .resource::<ProjectionSeed>()
        .pending_materials();
    assert!(persistent_seed_meshes > 0, "persistent mesh seeds");
    assert!(persistent_seed_materials > 0, "persistent material seeds");
    let persistent_live = usd_bevy::LiveStage::new(stage.clone());
    let mut persistent_map = usd_bevy::PrimEntities::default();
    usd_bevy::project_stage(
        persistent_app.world_mut(),
        &persistent_live,
        &mut persistent_map,
    );
    let persistent_ms = persistent_start.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(
        persistent_app
            .world()
            .resource::<ProjectionSeed>()
            .pending_meshes(),
        0,
        "persistent mesh seeds must be consumed by projection"
    );
    assert_eq!(
        persistent_app
            .world()
            .resource::<ProjectionSeed>()
            .pending_materials(),
        0,
        "persistent material seeds must be consumed by projection"
    );
    let persistent_projection = persistent_app
        .world()
        .resource::<usd_bevy::route::cache::ProjectionCache>()
        .stats();
    let persistent_material = persistent_app
        .world()
        .resource::<usd_bevy::route::material::UsdMaterialCache>()
        .stats();
    assert_eq!(
        persistent_projection.misses, 0,
        "persistent source mesh misses"
    );
    assert_eq!(
        persistent_material.misses, 0,
        "persistent material decode misses"
    );

    let hot_live = usd_bevy::LiveStage::new(stage);
    let mut hot_app = headless_cache_app();
    let mut hot_map = usd_bevy::PrimEntities::default();
    usd_bevy::project_stage(hot_app.world_mut(), &hot_live, &mut hot_map);
    hot_app
        .world_mut()
        .resource_mut::<usd_bevy::route::cache::ProjectionCache>()
        .reset_stats();
    hot_app
        .world_mut()
        .resource_mut::<usd_bevy::route::material::UsdMaterialCache>()
        .reset_stats();
    let hot_start = Instant::now();
    usd_bevy::project_stage(hot_app.world_mut(), &hot_live, &mut hot_map);
    let hot_ms = hot_start.elapsed().as_secs_f64() * 1_000.0;
    let hot_projection = hot_app
        .world()
        .resource::<usd_bevy::route::cache::ProjectionCache>()
        .stats();
    let hot_material = hot_app
        .world()
        .resource::<usd_bevy::route::material::UsdMaterialCache>()
        .stats();
    assert!(hot_projection.hits > 0, "hot source mesh cache hits");
    assert!(hot_material.hits > 0, "hot source material cache hits");

    eprintln!(
        "[owner-review-3-c8++] headless-cache benchmark: cold_source_ms={cold_ms:.3}, persistent_build_ms={persistent_build_ms:.3}, persistent_hydration_and_projection_ms={persistent_ms:.3}, hot_session_ms={hot_ms:.3}, cold_mesh_misses={}, cold_material_misses={}, persistent_mesh_seeds={persistent_seed_meshes}, persistent_material_seeds={persistent_seed_materials}, persistent_mesh_misses={}, persistent_material_misses={}, hot_mesh_hits={}, hot_material_hits={}",
        cold_projection.misses,
        cold_material.misses,
        persistent_projection.misses,
        persistent_material.misses,
        hot_projection.hits,
        hot_material.hits,
    );
    Ok(())
}

fn headless_cache_app() -> App {
    let mut app = App::new();
    app.add_plugins(usd_bevy::UsdPlugin);
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<Assets<Image>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app
}
