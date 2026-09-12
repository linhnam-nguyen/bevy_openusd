//! Project-specific LiveStage installation and empty-state invalidation.
use bevy::prelude::*;
use openusd::usd::Stage;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;
use usd_bevy::LiveStage;
use usd_model::HashDigest;
use crate::project::cache_contract::{
    ProjectCacheTarget, SceneCacheActivation,
};
use crate::project::cache_hydration::{
    ActiveProjectCacheContext, default_project_cache_config_hash, hydrate_project_cache,
};
use crate::viewport::api::CurrentHierarchyProjection;
use super::{
    RequestedAsset, Spawned, StageHandle, StageInfo, StagePresentationContext,
    lifecycle_invalidation,
};
use crate::viewport::session::{
    PendingSceneCacheRevalidation, SceneCacheOwnershipContext, SceneCachePresentation,
};
#[path = "lifecycle_scene_cache.rs"]
mod scene_cache;
pub(crate) use scene_cache::publish_scene_cache_presentation_before_stage_open;
#[path = "lifecycle_scene_presentation.rs"]
mod scene_presentation;

/// Installs a Stage that was already opened and validated by the Project
/// activation candidate. The opened Stage is moved directly into LiveStage so
/// runtime and checkpoint tests share the same activation boundary.
pub(crate) fn activate_open_stage_with_cache_context_for_generation(
    world: &mut World,
    path: std::path::PathBuf,
    stage: Stage,
    cache_context: Option<ActiveProjectCacheContext>,
    scene_cache: Option<SceneCacheActivation>,
    scene_cache_project_root: Option<std::path::PathBuf>,
    scene_owner_id: Option<usd_project::SceneId>,
    archive_paths: Option<Vec<std::path::PathBuf>>,
    activation_generation: u64,
    presentation: StagePresentationContext,
) -> Result<(), String> {
    let install_started = Instant::now();
    let root = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "resolved Project stage has no valid filename".to_owned())?
        .to_owned();
    let cache_context = cache_context.and_then(|context| {
        if !context.should_revalidate() {
            return Some(context);
        }
        let current = crate::project::cache::ProjectCacheIdentity::for_project(
            &context.project_root,
            context.identity.target.clone(),
            context.identity.profile,
            context.identity.config_hash,
        );
        match current {
            Ok(identity) if identity == context.identity => Some(context),
            Ok(_) => {
                bevy::log::warn!(
                    "[project-cache] source changed across Stage::open for {}; using source projection",
                    path.display()
                );
                None
            }
            Err(error) => {
                bevy::log::warn!(
                    "[project-cache] could not revalidate source across Stage::open for {}; using source projection: {error:#}",
                    path.display()
                );
                None
            }
        }
    });
    let archive_paths = archive_paths.unwrap_or_else(|| {
        usd_bevy::route::material::archive_paths_for_stage(&stage, &path).unwrap_or_else(|error| {
            bevy::log::warn!(
                "could not derive active USDZ packages for {}; embedded textures will use source fallback: {error:#}",
                path.display()
            );
            Vec::new()
        })
    });
    let archive_count = archive_paths.len();
    if let Some(mut cache) = world.get_resource_mut::<usd_bevy::route::material::UsdTextureCache>()
    {
        cache.replace_active_archives(archive_paths);
    }
    if let Some(mut seed) = world.get_resource_mut::<usd_bevy::ProjectionSeed>() {
        seed.clear();
    }
    if let Some(context) = cache_context.as_ref() {
        match hydrate_project_cache(world, context) {
            Ok(true) => info!("hydrated Project runtime cache for {}", path.display()),
            Ok(false) => bevy::log::debug!(
                "[project-cache] no ready cache for {}; continuing with source projection",
                path.display()
            ),
            Err(error) => {
                bevy::log::warn!(
                    "[project-cache] cache hydration failed for {}; continuing with source projection: {error:#}",
                    path.display()
                );
                if let Some(mut seed) = world.get_resource_mut::<usd_bevy::ProjectionSeed>() {
                    seed.clear();
                }
            }
        }
    }
    let scene_cache = scene_cache.filter(|activation| {
        scene_cache::is_current(scene_cache_project_root.as_deref(), activation)
    });
    let scene_cache_owner = scene_cache_project_root
        .as_ref()
        .zip(scene_owner_id.as_ref())
        .map(|(project_root, scene_id)| SceneCacheOwnershipContext {
            project_root: project_root.clone(),
            scene_id: scene_id.clone(),
            config_hash: scene_cache
                .as_ref()
                .map_or_else(default_project_cache_config_hash, |cache| {
                    cache.descriptor.config_hash
                }),
        });
    super::clear_projected_stage(world);
    lifecycle_invalidation::reset_derived_state(world, activation_generation);
    if let (Some(scene_cache), Some(project_root)) =
        (scene_cache.as_ref(), scene_cache_project_root.as_deref())
    {
        match crate::project::cache_scene_hydration::hydrate_scene_cache_payloads(
            world,
            project_root,
            scene_cache,
        ) {
            Ok(true) => info!(
                "hydrated Scene material/texture cache for {}",
                scene_cache.descriptor.scene_id
            ),
            Ok(false) => bevy::log::debug!(
                "[project-cache] no renderer Scene payloads for {}; continuing with source projection",
                scene_cache.descriptor.scene_id
            ),
            Err(error) => bevy::log::warn!(
                "[project-cache] Scene payload hydration failed for {}; continuing with source projection: {error:#}",
                scene_cache.descriptor.scene_id
            ),
        }
    }
    world.insert_resource(RequestedAsset { name, root });
    world.insert_resource(StageHandle {
        path: path.clone(),
        error: None,
    });
    super::prim_count::initialize_for_activation(
        world,
        &path,
        activation_generation,
        scene_cache_project_root
            .as_ref()
            .zip(scene_owner_id)
            .map(|(root, scene_id)| (root.clone(), scene_id)),
        scene_cache.as_ref(),
    );
    world.resource_mut::<Spawned>().0 = false;
    world.insert_resource(presentation);
    if let Some(context) = cache_context {
        world.insert_resource(context);
    } else {
        world.remove_resource::<ActiveProjectCacheContext>();
    }
    if let Some(owner) = scene_cache_owner {
        world.insert_resource(owner);
    } else {
        world.remove_resource::<SceneCacheOwnershipContext>();
    }
    if let Some(scene_cache) = scene_cache.as_ref() {
        scene_presentation::publish_scene_cache_presentation(
            world,
            scene_cache,
            scene_cache_project_root.as_deref(),
        );
    }
    world.insert_non_send(LiveStage::new(stage));
    if let (Some(scene_cache), Some(project_root)) =
        (scene_cache.as_ref(), scene_cache_project_root)
    {
        install_scene_cache_revalidation(
            world,
            project_root,
            scene_cache.descriptor.scene_id,
            activation_generation,
            scene_cache.descriptor.generation,
            scene_cache.descriptor.source_content_hash,
            scene_cache.descriptor.config_hash,
        );
    }
    info!(
        "[project-loading] live_stage_install_ms={:.3} archives={} target={}",
        install_started.elapsed().as_secs_f64() * 1_000.0,
        archive_count,
        path.display()
    );
    Ok(())
}
fn install_scene_cache_revalidation(
    world: &mut World,
    project_root: std::path::PathBuf,
    scene_id: usd_project::SceneId,
    activation_generation: u64,
    scene_generation: u64,
    expected_hash: Option<HashDigest>,
    config_hash: HashDigest,
) {
    let result = Arc::new(Mutex::new(None));
    let result_slot = Arc::clone(&result);
    let hash_target = ProjectCacheTarget::Scene {
        id: scene_id.to_string(),
    };
    let worker_root = project_root.clone();
    let worker_name = format!("scene-cache-revalidate-{scene_id}");
    let spawn_result = thread::Builder::new()
        .name(worker_name)
        .spawn(move || {
            let result =
                crate::project::source_closure::target_content_hash(&worker_root, &hash_target)
                    .map_err(|error| format!("{error:#}"));
            match result_slot.lock() {
                Ok(mut slot) => *slot = Some(result),
                Err(_) => bevy::log::warn!(
                    "[project-cache] Scene source revalidation result could not be published; canonical LiveStage remains authoritative"
                ),
            }
        });
    if let Err(error) = spawn_result {
        bevy::log::warn!(
            "[project-cache] Scene source revalidation worker could not start for {scene_id}: {error}; canonical LiveStage remains authoritative"
        );
        discard_scene_cache_presentation(world, scene_id, scene_generation);
        return;
    }
    world.insert_resource(PendingSceneCacheRevalidation {
        project_root,
        scene_id,
        activation_generation,
        scene_generation,
        expected_hash,
        config_hash,
        result,
    });
}
fn discard_scene_cache_presentation(world: &mut World, scene_id: usd_project::SceneId, generation: u64) {
    let matches = world
        .get_resource::<SceneCachePresentation>()
        .is_some_and(|presentation| {
            presentation.scene_id == scene_id && presentation.generation == generation
        });
    if !matches {
        return;
    }
    world.remove_resource::<SceneCachePresentation>();
    crate::viewport::residency::retire_scene_cache_resources(world);
    let projection_ready = world
        .get_resource::<usd_bevy::ProgressiveProjectionState>()
        .is_some_and(|state| state.readiness() == usd_bevy::ProjectionReadiness::Ready);
    if !projection_ready
        && let Some(mut projection) = world.get_resource_mut::<CurrentHierarchyProjection>()
    {
        *projection = CurrentHierarchyProjection::empty(viewport_protocol::HierarchySource::Prim, 0);
    }
}
/// Completes the background Scene source check without blocking the frame that
/// admitted the canonical Stage. A changed or unverifiable source invalidates
/// only the matching Scene generation; LiveStage remains authoritative.
pub(crate) fn poll_scene_cache_revalidation(world: &mut World) {
    let Some((project_root, scene_id, activation_generation, scene_generation, expected_hash, config_hash, result)) = world
        .get_resource::<PendingSceneCacheRevalidation>()
        .map(|pending| {
            (
                pending.project_root.clone(),
                pending.scene_id,
                pending.activation_generation,
                pending.scene_generation,
                pending.expected_hash,
                pending.config_hash,
                std::sync::Arc::clone(&pending.result),
            )
        })
    else {
        return;
    };
    let result = match result.lock() {
        Ok(mut slot) => slot.take(),
        Err(_) => {
            bevy::log::warn!(
                "[project-cache] Scene source revalidation result was poisoned; canonical LiveStage remains authoritative"
            );
            world.remove_resource::<PendingSceneCacheRevalidation>();
            discard_scene_cache_presentation(world, scene_id, scene_generation);
            return;
        }
    };
    let Some(result) = result else { return };

    let active_generation = world
        .get_resource::<StageInfo>()
        .map_or(0, |info| info.activation_generation);
    let matching_presentation = world
        .get_resource::<SceneCachePresentation>()
        .is_some_and(|presentation| {
            presentation.scene_id == scene_id && presentation.generation == scene_generation
        });
    if active_generation != activation_generation || !matching_presentation {
        world.remove_resource::<PendingSceneCacheRevalidation>();
        return;
    }

    let changed = match result {
        Ok(actual_hash) => expected_hash.is_some_and(|expected| expected != actual_hash),
        Err(error) => {
            bevy::log::warn!(
                "[project-cache] Scene source revalidation failed for {scene_id}: {error}"
            );
            true
        }
    };
    if changed {
        let store = crate::project::cache::SceneCacheStore::new(&project_root);
        match store.load_descriptor(scene_id) {
            Ok(Some(descriptor)) if descriptor.generation == scene_generation => {
                if let Err(error) = store.force_invalidate_generation(scene_id, config_hash) {
                    bevy::log::warn!(
                        "[project-cache] could not invalidate stale Scene cache generation {scene_generation} for {scene_id}: {error:#}"
                    );
                }
            }
            Ok(_) => {}
            Err(error) => bevy::log::warn!(
                "[project-cache] could not inspect Scene cache generation {scene_generation} for {scene_id}: {error:#}"
            ),
        }
        discard_scene_cache_presentation(world, scene_id, scene_generation);
        bevy::log::warn!(
            "[project-cache] rejected stale Scene metadata generation {scene_generation} for {scene_id}; canonical LiveStage projection continues"
        );
    } else {
        bevy::log::debug!(
            "[project-cache] Scene metadata generation {scene_generation} revalidated for {scene_id}"
        );
    }
    world.remove_resource::<PendingSceneCacheRevalidation>();
}

/// Clears the complete stage-scoped viewport state for an empty Project
/// activation before the caller commits its logical authority.
pub(crate) fn clear_active_stage_for_generation(world: &mut World, activation_generation: u64) {
    super::clear_projected_stage(world);
    if let Some(mut cache) = world.get_resource_mut::<usd_bevy::route::material::UsdTextureCache>()
    {
        cache.clear_active_archives();
    }
    lifecycle_invalidation::reset_derived_state(world, activation_generation);
    world.remove_resource::<RequestedAsset>();
    world.remove_resource::<StageHandle>();
    world.remove_resource::<ActiveProjectCacheContext>();
    world.remove_resource::<SceneCacheOwnershipContext>();
    world.insert_resource(StageInfo {
        activation_generation,
        ..StageInfo::default()
    });
    world.insert_resource(StagePresentationContext::default());
}
