//! Lifecycle for opening, projecting, and reloading a live USD stage.

use bevy::prelude::*;
use openusd::usd::Stage;
use usd_bevy::{AnimatedPrims, LiveStage, PathStore, PrimEntities};

use crate::project::cache_hydration::ActiveProjectCacheContext;

use super::{
    LoadRequest, ReloadRequest, RequestedAsset, Spawned, StageHandle, StageInfo,
    StagePresentationContext,
};

#[path = "lifecycle_invalidation.rs"]
mod lifecycle_invalidation;
#[path = "lifecycle_metadata.rs"]
mod lifecycle_metadata;
#[path = "lifecycle_open.rs"]
mod lifecycle_open;
#[path = "lifecycle_prim_count.rs"]
mod prim_count;
#[path = "lifecycle_project_activation.rs"]
mod project_activation;

pub(in crate::viewport) use lifecycle_invalidation::{
    PendingActivationPresentation, rehydrate_activation_presentation,
};
pub(crate) use project_activation::{
    StageInstallMode, activate_open_stage_with_cache_context_for_generation,
    clear_active_stage_for_generation, discard_scene_cache_bootstrap,
    install_scene_cache_bootstrap_before_stage_open, poll_scene_cache_revalidation,
};

const PROJECT_STAGE_OPEN_FAILURE: &str = "Project root stage could not be opened";

#[derive(Resource, Default, Debug, Clone, Copy)]
struct StageMetadataState {
    session_id: Option<u64>,
    complete: bool,
}

/// Open the requested file directly through the current OpenUSD API and place
/// it in `usd_bevy::LiveStage`. The new live plugin owns projection/revision
/// reconciliation; this system only owns the host-side open boundary.
pub(crate) fn load_stage(world: &mut World) {
    let requested = world.resource::<RequestedAsset>().clone();
    let path = requested.root.join(&requested.name);
    lifecycle_open::open_stage(world, path);
}

/// Opens a resolved Project root without disturbing the current stage if the
/// candidate cannot be opened. The caller owns identity resolution; this
/// function remains only a stage/lifecycle operation.
pub(crate) fn activate_stage(world: &mut World, path: std::path::PathBuf) -> Result<(), String> {
    activate_stage_with_cache_context(world, path, None)
}

/// Opens a canonical Project stage and opportunistically hydrates its exact
/// persistent runtime cache. Cache misses/corruption never prevent source
/// projection; the candidate Stage has already been opened successfully.
pub(crate) fn activate_stage_with_cache_context(
    world: &mut World,
    path: std::path::PathBuf,
    cache_context: Option<ActiveProjectCacheContext>,
) -> Result<(), String> {
    activate_stage_with_cache_context_inner(
        world,
        path,
        cache_context,
        0,
        StagePresentationContext::default(),
        || {},
    )
}
/// Opens a Project stage and records the activation generation for its snapshots.
pub(crate) fn activate_stage_with_cache_context_for_generation(
    world: &mut World,
    path: std::path::PathBuf,
    cache_context: Option<ActiveProjectCacheContext>,
    activation_generation: u64,
    presentation: StagePresentationContext,
) -> Result<(), String> {
    activate_stage_with_cache_context_inner(
        world,
        path,
        cache_context,
        activation_generation,
        presentation,
        || {},
    )
}

fn activate_stage_with_cache_context_inner<F>(
    world: &mut World,
    path: std::path::PathBuf,
    cache_context: Option<ActiveProjectCacheContext>,
    activation_generation: u64,
    presentation: StagePresentationContext,
    before_open: F,
) -> Result<(), String>
where
    F: FnOnce(),
{
    before_open();
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string).map_err(|error| {
        error!("failed to open Project stage {}: {error:#}", path.display());
        PROJECT_STAGE_OPEN_FAILURE.to_owned()
    })?;

    project_activation::activate_open_stage_with_cache_context_for_generation(
        world,
        path,
        stage,
        cache_context,
        None,
        None,
        None,
        None,
        activation_generation,
        presentation,
        project_activation::StageInstallMode::Fresh,
    )
}

/// Reload the current stage after a command or native keyboard request.
pub(crate) fn handle_usd_hot_reload(world: &mut World) {
    if !world.resource::<ReloadRequest>().requested {
        return;
    }
    world.resource_mut::<ReloadRequest>().requested = false;

    let path = {
        let requested = world.resource::<RequestedAsset>();
        requested.root.join(&requested.name)
    };
    clear_projected_stage(world);
    lifecycle_open::open_stage(world, path);
}

/// Marks a live stage ready once `LiveStagePlugin` has projected real prims
/// and captures the stage metadata used by the viewport read model.
pub(crate) fn spawn_when_ready(world: &mut World) {
    let Some((session_id, stage)) = world
        .get_non_send::<LiveStage>()
        .map(|live| (live.session_id(), live.stage.clone()))
    else {
        return;
    };
    prim_count::update(world, session_id);
    let projection_ready = world
        .get_resource::<usd_bevy::ProgressiveProjectionState>()
        .is_some_and(|state| state.readiness() == usd_bevy::ProjectionReadiness::Ready);
    let metadata_complete = world
        .get_resource::<StageMetadataState>()
        .is_some_and(|state| state.session_id == Some(session_id) && state.complete);
    if world.resource::<Spawned>().0 {
        if !projection_ready || metadata_complete {
            return;
        }
        world.resource_scope(|world, mut info: Mut<StageInfo>| {
            let map = world.resource::<PrimEntities>();
            let paths = world.resource::<PathStore>();
            lifecycle_metadata::refresh(&stage, map, paths, &mut info);
        });
        world.insert_resource(StageMetadataState {
            session_id: Some(session_id),
            complete: true,
        });
        return;
    }
    let prim_count = world.resource::<PrimEntities>().len();
    if prim_count <= 1 {
        return;
    }

    let animated_count = world
        .get_resource::<AnimatedPrims>()
        .map(|animated| animated.0.len())
        .unwrap_or_default();
    let default_prim = stage.default_prim().map(|prim| format!("/{prim}"));
    {
        let mut info = world.resource_mut::<StageInfo>();
        info.default_prim = default_prim.clone();
        info.layer_count = 1;
        info.animated_prim_count = animated_count;
        info.skel_animation_count = 0;
    }
    prim_count::publish_projected_count(world, session_id, prim_count.saturating_sub(1));

    world.resource_mut::<Spawned>().0 = true;
    world.insert_resource(StageMetadataState {
        session_id: Some(session_id),
        complete: projection_ready,
    });
    if projection_ready {
        world.resource_scope(|world, mut info: Mut<StageInfo>| {
            let map = world.resource::<PrimEntities>();
            let paths = world.resource::<PathStore>();
            lifecycle_metadata::refresh(&stage, map, paths, &mut info);
        });
    }
    info!(
        "live USD stage projected: {} prims ({} animated), default_prim={default_prim:?}",
        prim_count.saturating_sub(1),
        animated_count,
    );
}

/// Browse-USD re-launch remains host-owned because the viewer process owns the
/// stage path. The new process opens the selected path through `LiveStage` on
/// startup.
pub(crate) fn apply_load_request(mut request: ResMut<LoadRequest>) {
    let Some(path) = request.path.take() else {
        return;
    };
    let Ok(exe) = std::env::current_exe() else {
        error!("Browse: cannot resolve current executable");
        return;
    };
    match std::process::Command::new(exe).arg(path).spawn() {
        Ok(_) => std::process::exit(0),
        Err(error) => error!("Browse: failed to relaunch viewer: {error}"),
    }
}

pub(super) fn clear_live_stage_projection(world: &mut World) {
    let entities: Vec<Entity> = world
        .get_resource::<PrimEntities>()
        .zip(world.get_resource::<PathStore>())
        .map_or_else(Vec::new, |(map, paths)| {
            map.iter(paths).map(|(_, entity)| entity).collect()
        });
    for entity in entities {
        let _ = world.despawn(entity);
    }
    *world.resource_mut::<PrimEntities>() = PrimEntities::default();
    if let Some(mut provenance) =
        world.get_resource_mut::<usd_bevy::route::material::MaterialProjectionProvenance>()
    {
        provenance.clear();
    }
    if let Some(mut stage_time) = world.get_resource_mut::<usd_bevy::StageTime>() {
        stage_time.clear_stage();
    }
    world.remove_non_send::<LiveStage>();
    world.remove_resource::<StageMetadataState>();
    world.remove_resource::<super::PendingSceneCacheRevalidation>();
    world.resource_mut::<Spawned>().0 = false;
}

fn clear_projected_stage(world: &mut World) {
    clear_live_stage_projection(world);
    world.remove_resource::<super::SceneCachePresentation>();
    world.remove_resource::<crate::project::cache_contract::ProjectCacheLookup>();
    world.remove_resource::<super::SceneDerivedMetadata>();
    crate::viewport::residency::retire_scene_cache_resources(world);
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/session/lifecycle_cache_tests.rs"]
mod cache_tests;

#[cfg(test)]
#[path = "../../../tests/unit/viewport/session/lifecycle_inline_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests/unit/viewport/session/lifecycle_test_support.rs"]
mod test_support;
#[cfg(test)]
use test_support::activate_stage_with_cache_context_for_test;
