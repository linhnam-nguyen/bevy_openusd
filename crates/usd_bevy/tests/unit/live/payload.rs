use bevy::prelude::*;

use crate::live::{LiveStage, LiveStagePlugin, PathStore, PrimEntities};
use crate::snippet::UsdSnippet;

#[test]
fn test_live_stage_load_and_unload_payload_api_preserves_projection() {
    let usda = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/live/payload_1.usda"
    ));
    let stage = UsdSnippet::new(usda)
        .open_stage()
        .expect("payload stage opens");
    let live = LiveStage::new(stage);
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(live);
    app.update();

    let prim_entities = app.world().resource::<PrimEntities>();
    assert!(
        prim_entities
            .entity(app.world().resource::<PathStore>(), "/World/PayloadPrim")
            .is_some()
    );

    let live = app.world().get_non_send::<LiveStage>().unwrap();
    live.unload_payload("/World/PayloadPrim")
        .expect("unload payload succeeds");
    live.load_payload("/World/PayloadPrim")
        .expect("load payload succeeds");
}

#[test]
fn projects_skel_root_skeleton_and_skinning_data() {
    use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
    use openusd::usd::{PrimPredicate, SchemaBase};
    use openusd_schemas::skel::{BindingAPI as SkelBindingAPI, Cache, Root, Skeleton, SkeletonSchema};

    let stage = usd_support::open_fixture_stage("skel.usda");
    let mut app = App::new();
    app.add_plugins(crate::UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .add_plugins(bevy::transform::TransformPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<SkinnedMeshInverseBindposes>>();
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();

    let world = app.world_mut();
    let drivers: Vec<_> = world
        .query::<&crate::route::skel::UsdSkelAnimDriver>()
        .iter(world)
        .cloned()
        .collect();
    assert_eq!(drivers.len(), 1);
    assert_eq!(
        drivers[0].skeleton_joints,
        vec!["root".to_string(), "root/tip".to_string()]
    );
    assert_eq!(drivers[0].animation_source_path, "/World/Rig/Anim");

    let mut joint_paths: Vec<_> = world
        .query::<&crate::route::skel::UsdJoint>()
        .iter(world)
        .map(|j| (j.skeleton_path.clone(), j.path.clone()))
        .collect();
    joint_paths.sort();
    assert_eq!(
        joint_paths,
        vec![
            ("/World/Rig/Skel".to_string(), "root".to_string()),
            ("/World/Rig/Skel".to_string(), "root/tip".to_string()),
        ]
    );

    let live = world.get_non_send::<LiveStage>().expect("live stage");
    let root_path = openusd::sdf::path("/World/Rig").unwrap();
    let root = Root::get(&live.stage, root_path).unwrap().expect("SkelRoot");
    let mut cache = Cache::new();
    cache.populate(&root, PrimPredicate::DEFAULT).unwrap();
    let bindings = cache
        .skel_bindings(&root, PrimPredicate::DEFAULT)
        .expect("bindings discover");
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].skeleton().path().as_str(), "/World/Rig/Skel");
    assert_eq!(
        bindings[0]
            .skeleton()
            .animation_source()
            .unwrap()
            .as_ref()
            .map(|a| a.path().as_str()),
        Some("/World/Rig/Anim")
    );
    let arm_path = openusd::sdf::path("/World/Rig/Arm").unwrap();
    let skel_path = openusd::sdf::path("/World/Rig/Skel").unwrap();
    let skel = Skeleton::get(&live.stage, skel_path)
        .unwrap()
        .expect("skeleton exists");
    assert_eq!(
        skel.joints_attr().cast::<Vec<String>>().unwrap().unwrap_or_default(),
        vec!["root".to_string(), "root/tip".to_string()]
    );
    let binding_api = SkelBindingAPI::get(&live.stage, arm_path)
        .unwrap()
        .expect("Arm has SkelBindingAPI");
    let indices = match binding_api
        .joint_indices_attr()
        .get::<openusd::vt::Value>()
        .unwrap()
    {
        Some(openusd::vt::Value::IntVec(v)) => v,
        other => panic!("expected IntVec jointIndices, got {other:?}"),
    };
    let weights = match binding_api
        .joint_weights_attr()
        .get::<openusd::vt::Value>()
        .unwrap()
    {
        Some(openusd::vt::Value::FloatVec(v)) => v,
        other => panic!("expected FloatVec jointWeights, got {other:?}"),
    };
    assert_eq!(indices.len(), 6);
    assert_eq!(weights.len(), 6);
    assert_eq!(indices[0], 0);
    assert!((weights[0] - 1.0).abs() < 1e-5);
    assert!((weights[2] - 0.5).abs() < 1e-5);
    assert!((weights[3] - 0.5).abs() < 1e-5);
}
