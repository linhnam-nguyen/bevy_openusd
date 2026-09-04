//! Project-specific LiveStage installation and empty-state invalidation.

use bevy::prelude::*;
use openusd::usd::Stage;
use usd_bevy::LiveStage;

use crate::project::cache_hydration::{ActiveProjectCacheContext, hydrate_project_cache};

use super::{
    RequestedAsset, Spawned, StageHandle, StageInfo, StagePresentationContext,
    lifecycle_invalidation,
};

/// Installs a Stage that was already opened and validated by the Project
/// activation candidate. The opened Stage is moved directly into LiveStage so
/// runtime and checkpoint tests share the same activation boundary.
pub(crate) fn activate_open_stage_with_cache_context_for_generation(
    world: &mut World,
    path: std::path::PathBuf,
    stage: Stage,
    cache_context: Option<ActiveProjectCacheContext>,
    activation_generation: u64,
    presentation: StagePresentationContext,
) -> Result<(), String> {
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

    let archive_paths = usd_bevy::route::material::archive_paths_for_stage(&stage, &path)
        .map_err(|error| format!("derive active USDZ packages: {error:#}"))?;
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

    super::clear_projected_stage(world);
    lifecycle_invalidation::reset_derived_state(world, activation_generation);
    world.insert_resource(RequestedAsset { name, root });
    world.insert_resource(StageHandle {
        path: path.clone(),
        error: None,
    });
    world.resource_mut::<StageInfo>().path = path.to_string_lossy().into_owned();
    world.resource_mut::<StageInfo>().activation_generation = activation_generation;
    world.resource_mut::<Spawned>().0 = false;
    world.insert_resource(presentation);
    if let Some(context) = cache_context {
        world.insert_resource(context);
    } else {
        world.remove_resource::<ActiveProjectCacheContext>();
    }
    world.insert_non_send(LiveStage::new(stage));
    info!("activated Project USD stage: {}", path.display());
    Ok(())
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
    world.insert_resource(StageInfo {
        activation_generation,
        ..StageInfo::default()
    });
    world.insert_resource(StagePresentationContext::default());
}
