use std::time::Duration;

use bevy::prelude::*;
use openusd::usd::Stage;
use usd_bevy::{LiveStage, LiveStagePlugin, ProjectionBudget, ProjectionReadiness, UsdPlugin};

use super::{UsdStageTime, tick_stage_time};

fn asset_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets/external")
        .join(name)
}

fn stage_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/stages")
        .join(name)
}

fn playback_app() -> App {
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .add_plugins(bevy::transform::TransformPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
        .insert_resource(ProjectionBudget::unlimited())
        .insert_resource(Time::<()>::default())
        .init_resource::<UsdStageTime>()
        .add_systems(
            Update,
            tick_stage_time.after(usd_bevy::LiveStageSet::Reconcile),
        );
    app
}

fn open_stage(path: &std::path::Path) -> Stage {
    Stage::open(path.to_str().expect("fixture path is valid")).expect("stage opens")
}

fn settle(app: &mut App) {
    for _ in 0..512 {
        app.update();
        if app
            .world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness()
            == ProjectionReadiness::Ready
        {
            return;
        }
    }
    panic!("projection did not settle");
}

fn replace_stage(app: &mut App, stage: Stage) {
    app.world_mut()
        .get_non_send_mut::<LiveStage>()
        .expect("live stage")
        .replace_stage(stage);
    settle(app);
}

fn animated_transforms(app: &mut App) -> Vec<(String, Transform)> {
    let world = app.world_mut();
    let mut paths = world
        .resource::<usd_bevy::AnimatedPrims>()
        .0
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    paths.sort();
    let map = world.resource::<usd_bevy::PrimEntities>();
    let path_store = world.resource::<usd_bevy::PathStore>();
    let mut output = paths
        .into_iter()
        .filter_map(|path| {
            let entity = map.entity(path_store, &path)?;
            Some((path, *world.get::<Transform>(entity)?))
        })
        .collect::<Vec<_>>();
    let mut query = world.query::<(&usd_bevy::route::skel::UsdJoint, &Transform)>();
    output.extend(
        query
            .iter(world)
            .map(|(joint, transform)| (format!("joint:{}", joint.path), *transform)),
    );
    output
}

#[test]
fn hummingbird_replacement_resets_and_restarts_real_playback() {
    let mut app = playback_app();
    app.world_mut()
        .insert_non_send(LiveStage::new(open_stage(&asset_path("hummingbird.usdz"))));
    settle(&mut app);

    let (start, end, fps) = {
        let live = app.world().get_non_send::<LiveStage>().expect("live stage");
        (
            live.stage.start_time_code(),
            live.stage.end_time_code(),
            live.stage.time_codes_per_second(),
        )
    };
    assert!(
        end > start,
        "Hummingbird must expose an authored time range"
    );
    assert!(fps > 0.0);
    assert!(
        !app.world()
            .resource::<usd_bevy::AnimatedPrims>()
            .0
            .is_empty()
    );

    let initial_current = app.world().resource::<usd_bevy::StageTime>().current;
    assert!(app.world().resource::<UsdStageTime>().playing);
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_millis(100));
    app.update();
    assert_ne!(
        app.world().resource::<usd_bevy::StageTime>().current,
        initial_current,
        "real playback must advance StageTime"
    );

    {
        let mut clock = app.world_mut().resource_mut::<UsdStageTime>();
        clock.playing = false;
        clock.seconds = 0.0;
    }
    app.update();
    let at_start = animated_transforms(&mut app);
    assert!(
        !at_start.is_empty(),
        "animated Hummingbird transforms exist"
    );
    {
        let mut clock = app.world_mut().resource_mut::<UsdStageTime>();
        clock.seconds = (end - start) / fps / 2.0;
    }
    app.update();
    let at_mid = animated_transforms(&mut app);
    let changing_path = at_start.iter().find_map(|(path, start_transform)| {
        at_mid
            .iter()
            .find(|(mid_path, _)| mid_path == path)
            .and_then(|(_, mid_transform)| {
                (start_transform != mid_transform).then_some(path.as_str())
            })
    });
    assert!(
        changing_path.is_some(),
        "real Hummingbird animated transforms must change by sample"
    );

    replace_stage(&mut app, open_stage(&stage_path("hierarchy.usda")));
    assert!(
        app.world()
            .resource::<usd_bevy::AnimatedPrims>()
            .0
            .is_empty()
    );
    assert!(!app.world().resource::<UsdStageTime>().playing);
    assert_eq!(
        app.world().resource::<usd_bevy::StageTime>().current,
        open_stage(&stage_path("hierarchy.usda")).start_time_code()
    );

    replace_stage(&mut app, open_stage(&asset_path("hummingbird.usdz")));
    assert!(
        !app.world()
            .resource::<usd_bevy::AnimatedPrims>()
            .0
            .is_empty()
    );
    assert!(app.world().resource::<UsdStageTime>().playing);
    let before_restart = app.world().resource::<usd_bevy::StageTime>().current;
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_millis(100));
    app.update();
    assert_ne!(
        app.world().resource::<usd_bevy::StageTime>().current,
        before_restart,
        "Hummingbird playback must restart after static replacement"
    );
}
