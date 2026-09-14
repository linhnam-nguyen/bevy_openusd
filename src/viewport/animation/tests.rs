use bevy::prelude::*;
use openusd::usd::Stage;
use usd_bevy::{LiveStage, LiveStagePlugin, ProjectionBudget, ProjectionReadiness, UsdPlugin};

use super::{UsdStageTime, systems::tick_stage_time};

#[path = "tests/cache_first.rs"]
mod cache_first;

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
            tick_stage_time
                .after(usd_bevy::LiveStageSet::Reconcile)
                .before(usd_bevy::LiveStageSet::Animation),
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

fn seek_paused(app: &mut App, time_code: f64) -> f64 {
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
    current
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
    output.sort_by(|(left, _), (right, _)| left.cmp(right));
    output
}

#[derive(Debug, PartialEq, Eq)]
struct QuantizedTransform {
    translation: [i64; 3],
    rotation: [i64; 4],
    scale: [i64; 3],
}

#[derive(Debug, PartialEq, Eq)]
struct TransformSignature {
    samples: Vec<(String, QuantizedTransform)>,
    hash: u64,
}

fn quantize(value: f32) -> i64 {
    const SCALE: f64 = 1_000_000.0;
    assert!(
        value.is_finite(),
        "animation transform component must be finite"
    );
    (f64::from(value) * SCALE).round() as i64
}

fn quantized_transform(transform: &Transform) -> QuantizedTransform {
    QuantizedTransform {
        translation: transform.translation.to_array().map(quantize),
        rotation: transform.rotation.to_array().map(quantize),
        scale: transform.scale.to_array().map(quantize),
    }
}

fn fnv1a_update(hash: &mut u64, bytes: &[u8]) {
    const FNV_PRIME: u64 = 1_099_511_628_211;
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn transform_signature(samples: &[(String, Transform)]) -> TransformSignature {
    const FNV_OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
    let samples = samples
        .iter()
        .map(|(path, transform)| (path.clone(), quantized_transform(transform)))
        .collect::<Vec<_>>();
    let mut hash = FNV_OFFSET_BASIS;
    for (path, transform) in &samples {
        fnv1a_update(&mut hash, &(path.len() as u64).to_le_bytes());
        fnv1a_update(&mut hash, path.as_bytes());
        for component in transform
            .translation
            .iter()
            .chain(transform.rotation.iter())
            .chain(transform.scale.iter())
        {
            fnv1a_update(&mut hash, &component.to_le_bytes());
        }
    }
    TransformSignature { samples, hash }
}

#[test]
fn hummingbird_native_stage_evaluation_is_animated() {
    let mut app = playback_app();
    app.world_mut()
        .insert_non_send(LiveStage::new(open_stage(&asset_path("hummingbird.usdz"))));
    let stage = settle_stage(&mut app, true);
    assert!(stage.end > stage.start);
    assert!(stage.fps > 0.0);
    assert!(
        !app.world()
            .resource::<usd_bevy::AnimatedPrims>()
            .0
            .is_empty()
    );

    let [t0, t1] = authored_sample_points(stage);
    let time0 = seek_paused(&mut app, t0);
    let transform0 = transform_signature(&animated_transforms(&mut app));
    let time1 = seek_paused(&mut app, t1);
    let transform1 = transform_signature(&animated_transforms(&mut app));
    assert!((time0 - t0).abs() < 1e-9);
    assert!((time1 - t1).abs() < 1e-9);
    assert_ne!(transform0.hash, transform1.hash);
}

#[test]
fn static_stage_is_negative_animation_control() {
    let mut app = playback_app();
    app.world_mut()
        .insert_non_send(LiveStage::new(open_stage(&stage_path("hierarchy.usda"))));
    let stage = settle_stage(&mut app, false);

    assert!(
        app.world()
            .resource::<usd_bevy::AnimatedPrims>()
            .0
            .is_empty()
    );
    assert!(!app.world().resource::<UsdStageTime>().playing);
    assert_eq!(
        app.world().resource::<usd_bevy::StageTime>().current,
        stage.start
    );
}

#[test]
fn hummingbird_static_hummingbird_replacement_restores_animation() {
    let mut app = playback_app();
    app.world_mut()
        .insert_non_send(LiveStage::new(open_stage(&asset_path("hummingbird.usdz"))));
    let first = settle_stage(&mut app, true);

    let static_stage = replace_stage(&mut app, open_stage(&stage_path("hierarchy.usda")), false);
    assert_eq!(static_stage.identity.0, first.identity.0);
    assert!(static_stage.identity.1 > first.identity.1);
    assert!(
        app.world()
            .resource::<usd_bevy::AnimatedPrims>()
            .0
            .is_empty()
    );
    assert!(!app.world().resource::<UsdStageTime>().playing);

    let restarted = replace_stage(&mut app, open_stage(&asset_path("hummingbird.usdz")), true);
    assert_eq!(restarted.identity.0, first.identity.0);
    assert!(restarted.identity.1 > static_stage.identity.1);
    assert!(
        !app.world()
            .resource::<usd_bevy::AnimatedPrims>()
            .0
            .is_empty()
    );
    assert!(app.world().resource::<UsdStageTime>().playing);
}

#[test]
fn hummingbird_paused_seek_round_trip_has_stable_transform_signature() {
    let mut app = playback_app();
    app.world_mut()
        .insert_non_send(LiveStage::new(open_stage(&asset_path("hummingbird.usdz"))));
    let stage = settle_stage(&mut app, true);
    let [t0, t1] = authored_sample_points(stage);
    assert!(t1 > t0, "authored sample points must advance in time");

    let at_t0 = {
        let current = seek_paused(&mut app, t0);
        assert!((current - t0).abs() < 1e-9);
        let samples = animated_transforms(&mut app);
        assert!(!samples.is_empty(), "Hummingbird samples must be non-empty");
        transform_signature(&samples)
    };
    let at_t1 = {
        let current = seek_paused(&mut app, t1);
        assert!((current - t1).abs() < 1e-9);
        let samples = animated_transforms(&mut app);
        assert!(!samples.is_empty(), "Hummingbird samples must be non-empty");
        transform_signature(&samples)
    };
    assert_ne!(at_t0.hash, at_t1.hash, "t0 and t1 must render differently");
    assert_ne!(
        at_t0.samples, at_t1.samples,
        "t0 and t1 samples must differ"
    );

    let round_trip = {
        let current = seek_paused(&mut app, t0);
        assert!((current - t0).abs() < 1e-9);
        transform_signature(&animated_transforms(&mut app))
    };
    assert_eq!(at_t0, round_trip, "paused t0 seek must round-trip exactly");
}
