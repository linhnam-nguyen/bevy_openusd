//! Lifecycle for opening, projecting, and reloading a live USD stage.

use bevy::prelude::*;
use openusd::usd::{PrimPredicate, Stage};
use usd_bevy::{AnimatedPrims, LiveStage, PrimEntities};

use super::{
    LoadRequest, ReloadRequest, RequestedAsset, Spawned, StageCameraData, StageCameraInfo,
    StageCameraProjection, StageHandle, StageInfo, VariantSetInfo,
};

const PROJECT_STAGE_OPEN_FAILURE: &str = "Project root stage could not be opened";

/// Open the requested file directly through the current OpenUSD API and place
/// it in `usd_bevy::LiveStage`. The new live plugin owns projection/revision
/// reconciliation; this system only owns the host-side open boundary.
pub(crate) fn load_stage(world: &mut World) {
    let requested = world.resource::<RequestedAsset>().clone();
    let path = requested.root.join(&requested.name);
    open_stage(world, path);
}

/// Opens a resolved Project root without disturbing the current stage if the
/// candidate cannot be opened. The caller owns identity resolution; this
/// function remains only a stage/lifecycle operation.
pub(crate) fn activate_stage(world: &mut World, path: std::path::PathBuf) -> Result<(), String> {
    let root = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "resolved Project stage has no valid filename".to_owned())?
        .to_owned();
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string).map_err(|error| {
        error!("failed to open Project stage {}: {error:#}", path.display());
        PROJECT_STAGE_OPEN_FAILURE.to_owned()
    })?;

    if let Some(mut cache) = world.get_resource_mut::<usd_bevy::route::material::UsdTextureCache>()
        && !cache.archive_paths.contains(&path)
    {
        cache.archive_paths.push(path.clone());
    }
    clear_projected_stage(world);
    world.insert_resource(RequestedAsset { name, root });
    world.insert_resource(StageHandle {
        path: path.clone(),
        error: None,
    });
    world.resource_mut::<StageInfo>().path = path.to_string_lossy().into_owned();
    world.resource_mut::<Spawned>().0 = false;
    world.insert_non_send(LiveStage::new(stage));
    info!("activated Project USD stage: {}", path.display());
    Ok(())
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
        .resource::<PrimEntities>()
        .iter()
        .map(|(_, entity)| entity)
        .collect();
    for entity in entities {
        let _ = world.despawn(entity);
    }
    *world.resource_mut::<PrimEntities>() = PrimEntities::default();
    world.remove_non_send::<LiveStage>();
    world.resource_mut::<Spawned>().0 = false;
}

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
