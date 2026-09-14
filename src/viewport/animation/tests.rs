use std::time::Duration;

use bevy::prelude::*;
use openusd::usd::Stage;
use usd_bevy::{LiveStage, LiveStagePlugin, ProjectionBudget, ProjectionReadiness, UsdPlugin};

use super::{UsdStageTime, systems::tick_stage_time};

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

#[derive(Clone, Copy, Debug)]
struct ReadyStage {
    identity: (u64, u64),
    start: f64,
    end: f64,
    fps: f64,
}

fn settle_stage(app: &mut App, expect_animated: bool) -> ReadyStage {
    let expected_identity = app
        .world()
        .get_non_send::<LiveStage>()
        .expect("live stage")
        .stage_identity();
    for _ in 0..512 {
        app.update();
        let live_identity = app
            .world()
            .get_non_send::<LiveStage>()
            .expect("live stage")
            .stage_identity();
        let projection = app
            .world()
            .resource::<usd_bevy::ProgressiveProjectionState>();
        let stage_time_identity = app.world().resource::<UsdStageTime>().stage_identity();
        let has_animated_prims = !app
            .world()
            .resource::<usd_bevy::AnimatedPrims>()
            .0
            .is_empty();
        if live_identity == expected_identity
            && projection.readiness() == ProjectionReadiness::Ready
            && projection.session_id() == Some(expected_identity.0)
            && stage_time_identity == Some(expected_identity)
            && has_animated_prims == expect_animated
        {
            let live = app.world().get_non_send::<LiveStage>().expect("live stage");
            return ReadyStage {
                identity: expected_identity,
                start: live.stage.start_time_code(),
                end: live.stage.end_time_code(),
                fps: live.stage.time_codes_per_second(),
            };
        }
    }
    panic!("projection did not settle for stage identity {expected_identity:?}");
}

fn replace_stage(app: &mut App, stage: Stage, expect_animated: bool) -> ReadyStage {
    app.world_mut()
        .get_non_send_mut::<LiveStage>()
        .expect("live stage")
        .replace_stage(stage);
    settle_stage(app, expect_animated)
}

fn authored_sample_points(stage: ReadyStage) -> [f64; 2] {
    let span = stage.end - stage.start;
    [stage.start + span * 0.25, stage.start + span * 0.75]
}

fn seek_paused(app: &mut App, time_code: f64) {
    let seconds = {
        let clock = app.world().resource::<UsdStageTime>();
        (time_code - clock.start_time_code) / clock.time_codes_per_second
    };
    assert!(
        seconds.is_finite(),
        "paused seek must produce finite seconds"
    );
    {
        let mut clock = app.world_mut().resource_mut::<UsdStageTime>();
        clock.playing = false;
        clock.seconds = seconds;
    }
    app.update();
    let current = app.world().resource::<usd_bevy::StageTime>().current;
    assert!((current - time_code).abs() < 1e-9);
    assert!(!app.world().resource::<UsdStageTime>().playing);
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
    let hummingbird = settle_stage(&mut app, true);
    let (start, end, fps) = (hummingbird.start, hummingbird.end, hummingbird.fps);
    assert!(
        end > start,
        "Hummingbird must expose an authored time range"
    );
    assert!(fps > 0.0);

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

    let [t0, t1] = authored_sample_points(hummingbird);
    seek_paused(&mut app, t0);
    let at_t0 = animated_transforms(&mut app);
    assert!(!at_t0.is_empty(), "animated Hummingbird transforms exist");
    seek_paused(&mut app, t1);
    let at_t1 = animated_transforms(&mut app);
    let changing_path = at_t0.iter().find_map(|(path, t0_transform)| {
        at_t1
            .iter()
            .find(|(t1_path, _)| t1_path == path)
            .and_then(|(_, t1_transform)| (t0_transform != t1_transform).then_some(path.as_str()))
    });
    assert!(
        changing_path.is_some(),
        "real Hummingbird animated transforms must change between t0 and t1"
    );

    let static_stage = replace_stage(&mut app, open_stage(&stage_path("hierarchy.usda")), false);
    assert_eq!(static_stage.identity.0, hummingbird.identity.0);
    assert!(static_stage.identity.1 > hummingbird.identity.1);
    assert!(!app.world().resource::<UsdStageTime>().playing);
    assert_eq!(
        app.world().resource::<usd_bevy::StageTime>().current,
        static_stage.start
    );

    let restarted = replace_stage(&mut app, open_stage(&asset_path("hummingbird.usdz")), true);
    assert_eq!(restarted.identity.0, hummingbird.identity.0);
    assert!(restarted.identity.1 > static_stage.identity.1);
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
