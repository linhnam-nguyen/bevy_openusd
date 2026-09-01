//! Lifecycle for opening, projecting, and reloading a live USD stage.

use bevy::prelude::*;
use openusd::usd::{PrimPredicate, Stage};
use usd_bevy::{AnimatedPrims, LiveStage, PathStore, PrimEntities};

use super::{
    LoadRequest, ReloadRequest, RequestedAsset, Spawned, StageCameraData, StageCameraInfo,
    StageCameraProjection, StageHandle, StageInfo, VariantSetInfo,
};

/// Open the requested file directly through the current OpenUSD API and place
/// it in `usd_bevy::LiveStage`. The new live plugin owns projection/revision
/// reconciliation; this system only owns the host-side open boundary.
pub(crate) fn load_stage(world: &mut World) {
    let requested = world.resource::<RequestedAsset>().clone();
    let path = requested.root.join(&requested.name);
    open_stage(world, path);
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
    open_stage(world, path);
}

/// Marks a live stage ready once `LiveStagePlugin` has projected real prims
/// and captures the stage metadata used by the viewport read model.
pub(crate) fn spawn_when_ready(world: &mut World) {
    if world.resource::<Spawned>().0 {
        return;
    }
    let Some(live) = world.get_non_send::<LiveStage>() else {
        return;
    };
    let prim_count = world.resource::<PrimEntities>().len();
    if prim_count <= 1 {
        return;
    }

    let stage = live.stage.clone();
    let animated_count = world
        .get_resource::<AnimatedPrims>()
        .map(|animated| animated.0.len())
        .unwrap_or_default();
    let requested_path = {
        let requested = world.resource::<RequestedAsset>();
        requested.root.join(&requested.name)
    };
    let default_prim = stage.default_prim().map(|prim| format!("/{prim}"));
    let mut variants = std::collections::HashMap::new();
    let mut cameras = Vec::new();
    let _ = stage.traverse(PrimPredicate::DEFAULT, |path| {
        let prim = stage.prim(path.clone());
        if let Ok(Some(type_name)) = prim.type_name()
            && type_name.as_str() == "Camera"
        {
            cameras.push(StageCameraInfo {
                path: path.as_str().to_owned(),
                data: StageCameraData {
                    focal_length_mm: Some(50.0),
                    projection: Some(StageCameraProjection::Perspective),
                },
            });
        }
        if let Ok(selections) = prim.variant_sets().get_all_variant_selections()
            && !selections.is_empty()
        {
            variants.insert(
                path.as_str().to_owned(),
                selections
                    .into_iter()
                    .map(|(name, selection)| VariantSetInfo {
                        name,
                        selection: Some(selection),
                        options: Vec::new(),
                    })
                    .collect(),
            );
        }
    });
    let variant_count = variants.values().map(Vec::len).sum();
    {
        let mut info = world.resource_mut::<StageInfo>();
        info.path = requested_path.to_string_lossy().into_owned();
        info.default_prim = default_prim.clone();
        info.layer_count = 1;
        info.animated_prim_count = animated_count;
        info.variant_count = variant_count;
        info.variants = variants;
        info.cameras = cameras;
        info.skel_animation_count = 0;
    }

    world.resource_mut::<Spawned>().0 = true;
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

fn open_stage(world: &mut World, path: std::path::PathBuf) {
    world.remove_non_send::<LiveStage>();
    world.insert_resource(StageHandle {
        path: path.clone(),
        error: None,
    });
    world.resource_mut::<Spawned>().0 = false;

    if let Some(mut cache) = world.get_resource_mut::<usd_bevy::route::material::UsdTextureCache>()
        && !cache.archive_paths.contains(&path)
    {
        cache.archive_paths.push(path.clone());
    }

    info!("opening USD stage: {}", path.display());
    let path_string = path.to_string_lossy().into_owned();
    match Stage::open(&path_string) {
        Ok(stage) => world.insert_non_send(LiveStage::new(stage)),
        Err(error) => {
            let message = format!("failed to open {}: {error:#}", path.display());
            error!("{message}");
            world.resource_mut::<StageHandle>().error = Some(message);
        }
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
    world.remove_non_send::<LiveStage>();
    world.resource_mut::<Spawned>().0 = false;
}
