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
#[path = "lifecycle_project_activation.rs"]
mod project_activation;

pub(in crate::viewport) use lifecycle_invalidation::rehydrate_activation_presentation;
pub(crate) use project_activation::{
    activate_open_stage_with_cache_context_for_generation, clear_active_stage_for_generation,
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

#[cfg(test)]
fn activate_stage_with_cache_context_for_test<F>(
    world: &mut World,
    path: std::path::PathBuf,
    cache_context: Option<ActiveProjectCacheContext>,
    before_open: F,
) -> Result<(), String>
where
    F: FnOnce(),
{
    activate_stage_with_cache_context_inner(
        world,
        path,
        cache_context,
        0,
        StagePresentationContext::default(),
        before_open,
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
        activation_generation,
        presentation,
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
    let requested_path = {
        let requested = world.resource::<RequestedAsset>();
        requested.root.join(&requested.name)
    };
    let default_prim = stage.default_prim().map(|prim| format!("/{prim}"));
    {
        let mut info = world.resource_mut::<StageInfo>();
        info.path = requested_path.to_string_lossy().into_owned();
        info.default_prim = default_prim.clone();
        info.layer_count = 1;
        info.animated_prim_count = animated_count;
        info.skel_animation_count = 0;
    }

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

fn clear_projected_stage(world: &mut World) {
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
    world.resource_mut::<Spawned>().0 = false;
}

#[cfg(test)]
#[path = "lifecycle_cache_tests.rs"]
mod cache_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use project_protocol::{ProjectActivationCommand, ProjectActivationReply, ProjectStageTarget};
    use usd_project::{ProjectId, ProjectRoot};

    fn fixture_path(file_name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/stages")
            .join(file_name)
    }

    fn activation_world() -> World {
        let mut world = World::new();
        world.insert_resource(PrimEntities::default());
        world.insert_resource(Spawned::default());
        world.insert_resource(StageInfo::default());
        world
    }

    fn assert_active_stage(world: &World, expected: &std::path::Path) {
        let expected_string = expected.to_string_lossy().into_owned();
        let expected_root = expected.parent().unwrap().to_path_buf();
        let expected_name = expected.file_name().unwrap().to_string_lossy();
        let requested = world.resource::<RequestedAsset>();
        assert_eq!(requested.root, expected_root);
        assert_eq!(requested.name, expected_name.as_ref());
        assert_eq!(world.resource::<StageHandle>().path, expected);
        assert_eq!(world.resource::<StageInfo>().path, expected_string);
        assert!(
            world
                .get_non_send::<LiveStage>()
                .unwrap()
                .stage
                .layer_identifiers()
                .iter()
                .any(|identifier| identifier == &expected_string)
        );
    }

    #[test]
    fn project_activation_replaces_a_with_b_and_failed_b_preserves_a() {
        let scene_a = fixture_path("hierarchy.usda");
        let scene_b = fixture_path("primitives.usda");
        let missing_b = fixture_path("missing-project-root.usda");
        let mut world = activation_world();

        activate_stage(&mut world, scene_a.clone()).unwrap();
        let session_a = world.get_non_send::<LiveStage>().unwrap().session_id();
        assert_active_stage(&world, &scene_a);

        activate_stage(&mut world, scene_b.clone()).unwrap();
        let session_b = world.get_non_send::<LiveStage>().unwrap().session_id();
        assert_ne!(session_a, session_b);
        assert_active_stage(&world, &scene_b);

        activate_stage(&mut world, scene_a.clone()).unwrap();
        let preserved_session = world.get_non_send::<LiveStage>().unwrap().session_id();
        let error = activate_stage(&mut world, missing_b.clone()).unwrap_err();

        assert_eq!(error, PROJECT_STAGE_OPEN_FAILURE);
        assert_eq!(
            world.get_non_send::<LiveStage>().unwrap().session_id(),
            preserved_session
        );
        assert_active_stage(&world, &scene_a);

        let command = ProjectActivationCommand::new(
            "activation-failed",
            2,
            ProjectId::new_v4(),
            ProjectStageTarget::ProjectRoot(ProjectRoot::Empty),
        );
        let encoded =
            serde_json::to_string(&ProjectActivationReply::failed(&command, error)).unwrap();
        assert!(!encoded.contains(&missing_b.to_string_lossy().to_string()));
        assert!(!encoded.contains("missing-project-root.usda"));
        assert!(encoded.contains(PROJECT_STAGE_OPEN_FAILURE));
    }
}
