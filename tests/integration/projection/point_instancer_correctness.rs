//! M8-C2/C5 PointInstancer asset-reuse and logical-identity coverage.

use std::collections::HashSet;

use bevy::asset::{AssetId, Assets};
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use openusd::gf::Vec3f;
use openusd::sdf::Path;
use openusd::usd::Stage;
use openusd::vt::Value;
use usd_bevy::read::geom::read_point_instancer;
use usd_bevy::route::instancer::{PointInstancerRoute, UsdInstance, UsdInstanceId};
use usd_bevy::{
    LiveStage, PointInstancerSelection, PointInstancerStats, PrimRoute, RouteCtx, UsdPlugin,
};

const FIXTURE: &str = include_str!("../../fixtures/usd/point_instancer_prototypes.usda");
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
    let stage = usd_support::stage_from_usda(FIXTURE).expect("point instancer fixture opens");
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
    let mut query = world.query::<(&UsdInstanceId, &Mesh3d, &MeshMaterial3d<StandardMaterial>)>();
    let mut rows: Vec<_> = query.iter(world).collect();
    rows.sort_by_key(|(id, _, _)| id.source_index);
    let ids = rows.iter().map(|(id, _, _)| **id).collect();
    let mesh_handles = rows.iter().map(|(_, mesh, _)| mesh.0.id()).collect();
    let material_handles = rows
        .iter()
        .map(|(_, _, material)| material.0.id())
        .collect();
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

    // Some USD caches author an empty default and populate the first sample.
    // Opening them at the default time retains that existing projection behavior.
    let prim = stage.prim(path.clone()).unwrap();
    let positions = prim.attribute("positions");
    let authored = positions.get::<Value>().unwrap().unwrap();
    let time = openusd::usd::TimeCode::new(1.0);
    positions.clone().set_at(authored, time).unwrap();
    positions.set(Value::Vec3fVec(Vec::new())).unwrap();
    project_once(&mut app, &stage, &path, entity);
    assert_eq!(
        read_point_instancer(&stage, &path)
            .unwrap()
            .unwrap()
            .positions
            .len(),
        6
    );
    assert_eq!(snapshot(app.world_mut()).ids, result.ids);

    let equipment = Path::new("/Equipment/Instances").unwrap();
    assert_eq!(
        read_point_instancer(&stage, &equipment)
            .expect("equipment arrays read")
            .expect("equipment arrays exist")
            .ids,
        Some(vec![200, 201]),
    );
    let equipment_parent = app.world_mut().spawn_empty().id();
    project_once(&mut app, &stage, &equipment, equipment_parent);
    let occurrence = instance_entity(app.world_mut(), 200);
    let parts: Vec<_> = app
        .world()
        .get::<Children>(occurrence)
        .unwrap()
        .iter()
        .collect();
    assert_eq!(parts.len(), 2, "both equipment parts must be projected");
    let world = app.world();
    let housing = world
        .get::<MeshMaterial3d<StandardMaterial>>(parts[0])
        .unwrap();
    let panel = world
        .get::<MeshMaterial3d<StandardMaterial>>(parts[1])
        .unwrap();
    assert_ne!(housing.0, panel.0);
    assert_eq!(
        world.get::<Mesh3d>(parts[0]).unwrap().0,
        world.get::<Mesh3d>(parts[1]).unwrap().0
    );
    let local_to_occurrence = world
        .get::<usd_bevy::route::xform::UsdFullLocalAffine>(parts[1])
        .unwrap()
        .0;
    let occurrence_to_world = world.get::<Transform>(occurrence).unwrap().compute_affine();
    let world_point = (occurrence_to_world * local_to_occurrence).transform_point3(Vec3::Y);
    assert!(world_point.abs_diff_eq(Vec3::new(14.5, 1.0, 0.0), 1e-5));
    project_once(&mut app, &stage, &equipment, equipment_parent);
    let world = app.world();
    assert_eq!(
        world
            .get::<Children>(occurrence)
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        parts
    );
    stage
        .prim("/Equipment/Prototype/PanelMount")
        .unwrap()
        .attribute("visibility")
        .set(Value::Token("invisible".into()))
        .unwrap();
    project_once(&mut app, &stage, &equipment, equipment_parent);
    let world = app.world();
    assert!(world.get::<Mesh3d>(parts[0]).is_some());
    assert!(world.get::<Mesh3d>(parts[1]).is_none());
    assert_eq!(world.get::<Visibility>(parts[1]), Some(&Visibility::Hidden));
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
        .expect("cube prim exists")
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
    let mut query = world.query_filtered::<&UsdInstance, With<Mesh3d>>();
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
        .expect("instancer prim exists")
        .attribute("positions")
        .set(Value::Vec3fVec(positions))
        .expect("position reorder edit succeeds");
    let prim = stage.prim(path.clone()).unwrap();
    prim.attribute("ids")
        .set(Value::Int64Vec(vec![103, 100, 101, 102, 104, 105]))
        .unwrap();
    project_once(&mut app, &stage, &path, entity);

    let selected_after = instance_entity(app.world_mut(), 103);
    assert_eq!(
        selected_before, selected_after,
        "keyed reconciliation retains the authored logical instance entity"
    );
    assert_eq!(
        app.world().resource::<PointInstancerSelection>().logical_id,
        Some(103)
    );
    let children = app.world().get::<Children>(entity).unwrap();
    assert_eq!(children.iter().next(), Some(selected_before));

    prim.attribute("ids")
        .set(Value::Int64Vec(vec![103, 103, 101, 102, 104, 105]))
        .unwrap();
    project_once(&mut app, &stage, &path, entity);
    let children = app.world().get::<Children>(entity).unwrap();
    assert_eq!(children.iter().next(), Some(selected_before));
    assert_eq!(children.len(), 6);
    prim.attribute("ids")
        .set(Value::Int64Vec(vec![103, 100, 101, 102, 104, 105]))
        .unwrap();

    stage
        .prim(path.clone())
        .expect("instancer prim exists")
        .attribute("invisibleIds")
        .set(Value::Int64Vec(vec![103]))
        .expect("hidden-ID edit succeeds");
    project_once(&mut app, &stage, &path, entity);
    let world = app.world_mut();
    assert_eq!(instance_entity(world, 103), selected_before);
    assert_eq!(
        world.get::<Visibility>(selected_before),
        Some(&Visibility::Hidden)
    );
    assert!(world.get::<Mesh3d>(selected_before).is_none());

    // A removed ID may later be authored again, but cannot inherit the old
    // selection or the old representation.
    prim.attribute("ids")
        .set(Value::Int64Vec(vec![106, 100, 101, 102, 104, 105]))
        .unwrap();
    project_once(&mut app, &stage, &path, entity);
    assert!(app.world().get_entity(selected_before).is_err());
    assert_eq!(
        app.world().resource::<PointInstancerSelection>().logical_id,
        None
    );
    prim.attribute("ids")
        .set(Value::Int64Vec(vec![103, 100, 101, 102, 104, 105]))
        .unwrap();
    project_once(&mut app, &stage, &path, entity);
    assert_ne!(instance_entity(app.world_mut(), 103), selected_before);

    prim.attribute("invisibleIds")
        .set(Value::Int64Vec(Vec::new()))
        .unwrap();
    prim.clone()
        .set_metadata(
            "inactiveIds",
            Value::Int64ListOp(openusd::sdf::ListOp::explicit(vec![103])),
        )
        .unwrap();
    project_once(&mut app, &stage, &path, entity);
    let masked = instance_entity(app.world_mut(), 103);
    assert_eq!(
        app.world().get::<Visibility>(masked),
        Some(&Visibility::Hidden)
    );
    prim.clone().clear_metadata("inactiveIds").unwrap();
    project_once(&mut app, &stage, &path, entity);
    assert_eq!(instance_entity(app.world_mut(), 103), masked);
    assert!(app.world().get::<Mesh3d>(masked).is_some());

    prim.attribute("ids").clear().unwrap();
    project_once(&mut app, &stage, &path, entity);
    assert!(app.world().get_entity(masked).is_err());
    let anonymous = instance_entity(app.world_mut(), 0);
    project_once(&mut app, &stage, &path, entity);
    assert_eq!(instance_entity(app.world_mut(), 0), anonymous);
    prim.attribute("protoIndices")
        .set(Value::IntVec(vec![1, 0, 0, 1, 0, 1]))
        .unwrap();
    project_once(&mut app, &stage, &path, entity);
    assert_ne!(instance_entity(app.world_mut(), 0), anonymous);
}

fn live_app(stage: Stage) -> App {
    viewport_support::live_test_app(stage)
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
        .expect("cube prim exists")
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
fn live_prototype_ancestor_resync_reprojects_registered_instancer_dependency() {
    let (stage, _path) = open_fixture();
    let mut app = live_app(stage);
    let cube_before = mesh_handle_for_logical(app.world_mut(), 100);
    *app.world_mut().resource_mut::<PointInstancerStats>() = PointInstancerStats::default();

    let live = app
        .world()
        .get_non_send::<LiveStage>()
        .expect("live stage exists");
    live.stage
        .prim(Path::new(CUBE_MESH).expect("mesh path is valid"))
        .expect("cube prim exists")
        .attribute("points")
        .set(Value::Vec3fVec(vec![
            Vec3f::from([-2.0, -0.5, 0.0]),
            Vec3f::from([0.5, -0.5, 0.0]),
            Vec3f::from([0.5, 0.5, 0.0]),
            Vec3f::from([-2.0, 0.5, 0.0]),
        ]))
        .expect("prototype points edit succeeds");
    live.enqueue_resync("/World/Prototypes");
    app.update();

    let stats = *app.world().resource::<PointInstancerStats>();
    assert_eq!(
        stats.full_projects, 1,
        "ancestor resync reaches the consumer"
    );
    assert_eq!(stats.sparse_transform_patches, 0);
    assert_ne!(mesh_handle_for_logical(app.world_mut(), 100), cube_before);
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
        .expect("instancer prim exists")
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
