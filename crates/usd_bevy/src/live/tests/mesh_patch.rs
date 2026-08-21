use bevy::asset::Assets;
use bevy::prelude::*;
use openusd::gf::Vec3f;
use openusd::sdf::Value;

use crate::live::{
    LiveRevision, LiveStage, LiveStagePlugin, PrimEntities, StageChange, StageChangeBatch,
    apply_change_batch,
};
use crate::{GeometryProfile, UsdPlugin, UsdSnippet};

const MESH_PATH: &str = "/World/Triangle";

fn mesh_stage() -> openusd::usd::Stage {
    UsdSnippet::new(
        r#"#usda 1.0
def Xform "World"
{
    def Mesh "Triangle"
    {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        color3f[] primvars:displayColor = [(1, 0, 0), (0, 1, 0), (0, 0, 1)] (
            interpolation = "vertex"
        )
        float3[] extent = [(0, 0, 0), (1, 1, 0)]
    }
}
"#,
    )
    .open_stage()
    .expect("live mesh patch stage opens")
}

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(UsdPlugin);
    app.add_plugins(LiveStagePlugin);
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.world_mut().resource_mut::<GeometryProfile>().enabled = true;
    app.world_mut()
        .insert_non_send(LiveStage::new(mesh_stage()));
    app.update();
    app
}

fn mesh_handle(app: &App) -> Handle<Mesh> {
    let entity = app
        .world()
        .resource::<PrimEntities>()
        .entity(MESH_PATH)
        .expect("triangle entity exists");
    app.world()
        .get::<Mesh3d>(entity)
        .expect("triangle mesh exists")
        .0
        .clone()
}

fn mesh_totals(app: &App) -> crate::route::profile::GeometryProfileTotals {
    app.world().resource::<GeometryProfile>().totals
}

fn apply_changed_info(app: &mut App, property: &str) {
    apply_batch(
        app,
        StageChangeBatch {
            revision: LiveRevision(100),
            changes: vec![StageChange {
                resynced: Vec::new(),
                changed_info: vec![format!("{MESH_PATH}.{property}")],
            }],
        },
    );
}

fn apply_batch(app: &mut App, batch: StageChangeBatch) {
    let live = app
        .world_mut()
        .remove_non_send::<LiveStage>()
        .expect("live stage exists");
    let mut map = app
        .world_mut()
        .remove_resource::<PrimEntities>()
        .expect("prim map exists");
    apply_change_batch(app.world_mut(), &live, &mut map, &batch);
    app.world_mut().insert_resource(map);
    app.world_mut().insert_non_send(live);
}

fn set_points(app: &mut App) {
    let live = app
        .world()
        .get_non_send::<LiveStage>()
        .expect("live stage exists");
    let path = openusd::sdf::path(MESH_PATH).expect("mesh path is valid");
    live.stage
        .prim(path)
        .attribute("points")
        .set(Value::Vec3fVec(vec![
            Vec3f::from([0.0, 0.0, 0.0]),
            Vec3f::from([2.0, 0.0, 0.0]),
            Vec3f::from([0.0, 2.0, 0.0]),
        ]))
        .expect("points authoring succeeds");
    let _ = live.drain_change_batch();
}

fn set_display_color(app: &mut App) {
    let live = app
        .world()
        .get_non_send::<LiveStage>()
        .expect("live stage exists");
    let path = openusd::sdf::path(MESH_PATH).expect("mesh path is valid");
    live.stage
        .prim(path)
        .attribute("primvars:displayColor")
        .set(Value::Vec3fVec(vec![
            Vec3f::from([0.0, 1.0, 1.0]),
            Vec3f::from([1.0, 1.0, 0.0]),
            Vec3f::from([1.0, 0.0, 1.0]),
        ]))
        .expect("display color authoring succeeds");
    let _ = live.drain_change_batch();
}

fn add_subtree_mesh(app: &mut App) {
    let live = app
        .world()
        .get_non_send::<LiveStage>()
        .expect("live stage exists");
    let mesh = live.stage.define_prim("/World/NewTriangle").unwrap();
    mesh.create_attribute("faceVertexCounts", "int[]")
        .unwrap()
        .set(Value::IntVec(vec![3]))
        .unwrap();
    mesh.create_attribute("faceVertexIndices", "int[]")
        .unwrap()
        .set(Value::IntVec(vec![0, 1, 2]))
        .unwrap();
    mesh.create_attribute("points", "point3f[]")
        .unwrap()
        .set(Value::Vec3fVec(vec![
            Vec3f::from([0.0, 0.0, 0.0]),
            Vec3f::from([1.0, 0.0, 0.0]),
            Vec3f::from([0.0, 1.0, 0.0]),
        ]))
        .unwrap();
    let _ = live.drain_change_batch();
}

#[test]
fn m5_c4_live_patch_matrix_keeps_unrelated_edits_out_of_mesh_conversion() {
    let mut app = build_app();
    let initial_handle = mesh_handle(&app);
    let initial = mesh_totals(&app);

    for property in [
        "xformOp:translate",
        "visibility",
        "material:binding",
        "kind",
    ] {
        apply_changed_info(&mut app, property);
        assert_eq!(
            mesh_totals(&app).mesh_count,
            initial.mesh_count,
            "{property} must not invoke MeshRoute"
        );
        assert_eq!(
            mesh_handle(&app),
            initial_handle,
            "{property} keeps mesh handle"
        );
    }

    set_points(&mut app);
    apply_changed_info(&mut app, "points");
    let after_points = mesh_totals(&app);
    assert_eq!(after_points.mesh_count, initial.mesh_count + 1);
    assert_eq!(after_points.cache_misses, initial.cache_misses + 1);
    assert_ne!(mesh_handle(&app), initial_handle, "points replace the mesh");

    set_display_color(&mut app);
    apply_changed_info(&mut app, "primvars:displayColor");
    let after_primvar = mesh_totals(&app);
    assert_eq!(after_primvar.mesh_count, after_points.mesh_count + 1);
    assert_eq!(after_primvar.cache_misses, after_points.cache_misses + 1);

    add_subtree_mesh(&mut app);
    apply_batch(
        &mut app,
        StageChangeBatch {
            revision: LiveRevision(101),
            changes: vec![StageChange {
                resynced: vec!["/World/NewTriangle".to_string()],
                changed_info: Vec::new(),
            }],
        },
    );
    let after_add = mesh_totals(&app);
    assert_eq!(after_add.mesh_count, after_primvar.mesh_count + 1);
    assert!(
        app.world()
            .resource::<PrimEntities>()
            .entity("/World/NewTriangle")
            .is_some(),
        "subtree add is projected"
    );

    let live = app
        .world()
        .get_non_send::<LiveStage>()
        .expect("live stage exists");
    live.stage.remove_prim("/World/NewTriangle").unwrap();
    let _ = live.drain_change_batch();
    live.enqueue_resync("/World/NewTriangle");
    app.update();
    assert!(
        app.world()
            .resource::<PrimEntities>()
            .entity("/World/NewTriangle")
            .is_none(),
        "subtree removal is reconciled"
    );

    println!(
        "M5-C4 live patch matrix: initial_meshes={} final_meshes={} conversions_for_unrelated=0",
        initial.mesh_count,
        mesh_totals(&app).mesh_count
    );
}
