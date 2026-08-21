//! M8-C2/C5 PointInstancer asset-reuse and logical-identity coverage.

use std::collections::HashSet;
use std::path::PathBuf;

use bevy::asset::{AssetId, Assets};
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use openusd::gf::Vec3f;
use openusd::sdf::{Path, Value};
use openusd::usd::Stage;
use usd_bevy::read::geom::read_point_instancer;
use usd_bevy::route::instancer::{PointInstancerRoute, UsdInstance, UsdInstanceId};
use usd_bevy::{
    LiveStage, LiveStagePlugin, PointInstancerSelection, PointInstancerStats, PrimRoute, RouteCtx,
    UsdPlugin,
};

const FIXTURE: &str = "tests/stages/m8_point_instancer.usda";
const INSTANCER: &str = "/World/Instances";
const CUBE_MESH: &str = "/World/Prototypes/CubeProto/Mesh";

#[derive(Debug)]
struct InstanceSnapshot {
    ids: Vec<UsdInstanceId>,
    mesh_handles: Vec<AssetId<Mesh>>,
    material_handles: Vec<AssetId<StandardMaterial>>,
    mesh_assets: usize,
    material_assets: usize,
}

fn open_fixture() -> (Stage, Path) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
    let stage = Stage::open(path.to_str().expect("fixture path is valid")).expect("fixture opens");
    let instancer = Path::new(INSTANCER).expect("instancer path is valid");
    (stage, instancer)
}

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(UsdPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app
}

fn project_once(app: &mut App, stage: &Stage, path: &Path, entity: Entity) {
    let route = PointInstancerRoute;
    let ctx = RouteCtx::new(stage, path);
    route.project(&ctx, app.world_mut(), entity);
}

fn snapshot(world: &mut World) -> InstanceSnapshot {
    let mut ids = Vec::new();
    let mut mesh_handles = Vec::new();
    let mut material_handles = Vec::new();
    let mut query = world.query::<(
        &UsdInstanceId,
        Option<&Mesh3d>,
        Option<&MeshMaterial3d<StandardMaterial>>,
    )>();
    for (id, mesh, material) in query.iter(world) {
        ids.push(*id);
        mesh_handles.extend(mesh.map(|handle| handle.0.id()));
        material_handles.extend(material.map(|handle| handle.0.id()));
    }
    ids.sort_by_key(|id| id.source_index);
    let mesh_assets = world.resource::<Assets<Mesh>>().iter().count();
    let material_assets = world.resource::<Assets<StandardMaterial>>().iter().count();
    InstanceSnapshot {
        ids,
        mesh_handles,
        material_handles,
        mesh_assets,
        material_assets,
    }
}

#[test]
fn shared_prototypes_preserve_logical_order_and_invisible_ids() {
    let (stage, path) = open_fixture();
    let mut app = build_app();
    let entity = app.world_mut().spawn_empty().id();
    project_once(&mut app, &stage, &path, entity);

    let result = snapshot(app.world_mut());
    assert_eq!(
        result.ids,
        vec![
            UsdInstanceId {
                logical_id: 100,
                source_index: 0,
                prototype_index: 0
            },
            UsdInstanceId {
                logical_id: 102,
                source_index: 2,
                prototype_index: 0
            },
            UsdInstanceId {
                logical_id: 103,
                source_index: 3,
                prototype_index: 1
            },
            UsdInstanceId {
                logical_id: 104,
                source_index: 4,
                prototype_index: 0
            },
            UsdInstanceId {
                logical_id: 105,
                source_index: 5,
                prototype_index: 1
            },
        ]
    );
    assert_eq!(
        result.mesh_handles.len(),
        5,
        "every visible row has geometry"
    );
    assert_eq!(
        result.mesh_handles.iter().collect::<HashSet<_>>().len(),
        2,
        "prototype indices reuse two mesh assets"
    );
    assert_eq!(
        result.material_handles.iter().collect::<HashSet<_>>().len(),
        1,
        "fallback material is shared"
    );
    assert_eq!(result.mesh_assets, 2);
    assert_eq!(result.material_assets, 1);
}

#[test]
fn reprojection_reuses_assets_and_changed_prototype_gets_new_mesh() {
    let (stage, path) = open_fixture();
    let mut app = build_app();
    let entity = app.world_mut().spawn_empty().id();

    project_once(&mut app, &stage, &path, entity);
    let first = snapshot(app.world_mut());
    project_once(&mut app, &stage, &path, entity);
    let second = snapshot(app.world_mut());
    assert_eq!(first.ids, second.ids);
    assert_eq!(first.mesh_handles, second.mesh_handles);
    assert_eq!(first.material_handles, second.material_handles);
    assert_eq!(first.mesh_assets, second.mesh_assets);
    assert_eq!(first.material_assets, second.material_assets);

    stage
        .prim(Path::new(CUBE_MESH).expect("mesh path is valid"))
        .attribute("points")
        .set(Value::Vec3fVec(vec![
            Vec3f::from([-1.0, -0.5, 0.0]),
            Vec3f::from([0.5, -0.5, 0.0]),
            Vec3f::from([0.5, 0.5, 0.0]),
            Vec3f::from([-1.0, 0.5, 0.0]),
        ]))
        .expect("prototype edit succeeds");
    project_once(&mut app, &stage, &path, entity);
    let changed = snapshot(app.world_mut());
    assert_ne!(first.mesh_handles[0], changed.mesh_handles[0]);
    assert_eq!(
        changed.mesh_assets, 3,
        "changed geometry is a new cache entry"
    );
    assert_eq!(changed.material_assets, 1);
}

#[test]
fn point_instancer_marker_count_matches_visible_logical_rows() {
    let (stage, path) = open_fixture();
    let mut app = build_app();
    let entity = app.world_mut().spawn_empty().id();
    project_once(&mut app, &stage, &path, entity);

    let world = app.world_mut();
    let mut query = world.query::<&UsdInstance>();
    assert_eq!(query.iter(world).count(), 5);
}

fn instance_entity(world: &mut World, logical_id: i64) -> Entity {
    let mut query = world.query::<(Entity, &UsdInstanceId)>();
    query
        .iter(world)
        .find_map(|(entity, id)| (id.logical_id == logical_id).then_some(entity))
        .expect("logical instance is projected")
}

#[test]
fn logical_selection_survives_reprojection_and_hidden_ids_remove_pick_target() {
    let (stage, path) = open_fixture();
    let mut app = build_app();
    let entity = app.world_mut().spawn_empty().id();
    project_once(&mut app, &stage, &path, entity);

    let selected_before = instance_entity(app.world_mut(), 103);
    app.world_mut()
        .resource_mut::<PointInstancerSelection>()
        .select(INSTANCER, 103);

    let positions = vec![
        Vec3f::from([30.0, 0.0, 0.0]),
        Vec3f::from([20.0, 0.0, 0.0]),
        Vec3f::from([10.0, 0.0, 0.0]),
        Vec3f::from([0.0, 0.0, 0.0]),
        Vec3f::from([-10.0, 0.0, 0.0]),
        Vec3f::from([-20.0, 0.0, 0.0]),
    ];
    stage
        .prim(path.clone())
        .attribute("positions")
        .set(Value::Vec3fVec(positions))
        .expect("position reorder edit succeeds");
    project_once(&mut app, &stage, &path, entity);

    let selected_after = instance_entity(app.world_mut(), 103);
    assert_ne!(
        selected_before, selected_after,
        "full reproject replaces the child entity"
    );
    assert_eq!(
        app.world().resource::<PointInstancerSelection>().logical_id,
        Some(103)
    );

    stage
        .prim(path.clone())
        .attribute("invisibleIds")
        .set(Value::Int64Vec(vec![103]))
        .expect("hidden-ID edit succeeds");
    project_once(&mut app, &stage, &path, entity);
    let world = app.world_mut();
    let mut ids = world.query::<&UsdInstanceId>();
    assert!(
        ids.iter(world).all(|id| id.logical_id != 103),
        "hidden logical IDs must have no rendered or pickable entity"
    );
}

fn live_app(stage: Stage) -> App {
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();
    app
}

fn mesh_handle_for_logical(world: &mut World, logical_id: i64) -> AssetId<Mesh> {
    let mut query = world.query::<(&UsdInstanceId, &Mesh3d)>();
    query
        .iter(world)
        .find_map(|(id, mesh)| (id.logical_id == logical_id).then_some(mesh.0.id()))
        .expect("logical instance mesh exists")
}

#[test]
fn live_prototype_edit_reprojects_only_registered_instancer_dependency() {
    let (stage, _path) = open_fixture();
    let mut app = live_app(stage);
    let cube_before = mesh_handle_for_logical(app.world_mut(), 100);
    let sphere_before = mesh_handle_for_logical(app.world_mut(), 103);
    *app.world_mut().resource_mut::<PointInstancerStats>() = PointInstancerStats::default();

    let live = app
        .world()
        .get_non_send::<LiveStage>()
        .expect("live stage exists");
    live.stage
        .prim(Path::new(CUBE_MESH).expect("mesh path is valid"))
        .attribute("points")
        .set(Value::Vec3fVec(vec![
            Vec3f::from([-1.0, -0.5, 0.0]),
            Vec3f::from([0.5, -0.5, 0.0]),
            Vec3f::from([0.5, 0.5, 0.0]),
            Vec3f::from([-1.0, 0.5, 0.0]),
        ]))
        .expect("prototype points edit succeeds");
    app.update();

    let stats = *app.world().resource::<PointInstancerStats>();
    assert_eq!(
        stats.full_projects, 1,
        "only the dependent instancer reprojects"
    );
    assert_eq!(stats.sparse_transform_patches, 0);
    assert_ne!(mesh_handle_for_logical(app.world_mut(), 100), cube_before);
    assert_eq!(mesh_handle_for_logical(app.world_mut(), 103), sphere_before);
}

#[test]
fn live_transform_patch_preserves_logical_selection_and_entity() {
    let (stage, path) = open_fixture();
    let mut app = live_app(stage);
    let before = instance_entity(app.world_mut(), 103);
    app.world_mut()
        .resource_mut::<PointInstancerSelection>()
        .select(INSTANCER, 103);
    *app.world_mut().resource_mut::<PointInstancerStats>() = PointInstancerStats::default();

    let live = app
        .world()
        .get_non_send::<LiveStage>()
        .expect("live stage exists");
    let mut positions = read_point_instancer(&live.stage, &path)
        .expect("PointInstancer read succeeds")
        .expect("PointInstancer exists")
        .positions;
    positions[3][1] += 2.0;
    live.stage
        .prim(path)
        .attribute("positions")
        .set(Value::Vec3fVec(
            positions.into_iter().map(Vec3f::from).collect(),
        ))
        .expect("transform edit succeeds");
    app.update();

    let after = instance_entity(app.world_mut(), 103);
    let stats = *app.world().resource::<PointInstancerStats>();
    assert_eq!(
        before, after,
        "transform-only live patch retains the child entity"
    );
    assert_eq!(
        app.world().resource::<PointInstancerSelection>().logical_id,
        Some(103)
    );
    assert_eq!(stats.instance_spawns, 0);
    assert_eq!(stats.instance_despawns, 0);
    assert_eq!(stats.transform_updates, 5);
}
