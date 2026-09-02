use bevy::asset::Assets;
use bevy::image::Image;
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use openusd::gf::Vec3f;
use openusd::sdf::Value;
use openusd::usd::Stage;

use crate::ProjectionSeed;
use crate::UsdPlugin;
use crate::live::{
    LiveRevision, LiveStage, LiveStagePlugin, PathStore, PrimEntities, StageChange,
    StageChangeBatch, apply_change_batch,
};
use crate::route::material::MaterialRouteDiagnostics;

const RED_BOX: &str = "/World/RedBox";
const RED: &str = "/World/Materials/Red";
const GREEN: &str = "/World/Materials/GreenMetal";
const NETWORK_RED_BOX: &str = "/World/RedBox";
const NETWORK_RED_BALL: &str = "/World/RedBall";
const NETWORK_GREEN: &str = "/World/GreenCylinder";
const NETWORK_RED_SURFACE: &str = "/World/SharedShaders/RedSurface";
const NETWORK_RED_ALBEDO: &str = "/World/SharedShaders/RedAlbedo";

fn build_app() -> App {
    build_app_for("materials.usda")
}

fn build_app_for(fixture: &str) -> App {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stages")
        .join(fixture);
    let stage = Stage::open(path.to_str().expect("fixture path is valid"))
        .expect("materials fixture opens");
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();
    app
}

#[test]
fn persistent_material_seed_is_consumed_before_usd_decode() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stages")
        .join("materials.usda");
    let stage = Stage::open(path.to_str().expect("fixture path is valid"))
        .expect("materials fixture opens");
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>();
    let seeded = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.12, 0.34, 0.56),
            ..Default::default()
        });
    app.world_mut()
        .resource_mut::<ProjectionSeed>()
        .insert_material(RED_BOX, seeded.clone());
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();

    assert_eq!(material_handle(&app, RED_BOX), seeded);
    assert_eq!(
        app.world().resource::<ProjectionSeed>().pending_materials(),
        0
    );
}

fn entity(app: &App, path: &str) -> Entity {
    app.world()
        .resource::<PrimEntities>()
        .entity(app.world().resource::<PathStore>(), path)
        .unwrap_or_else(|| panic!("{path} entity exists"))
}

fn material_handle(app: &App, path: &str) -> Handle<StandardMaterial> {
    app.world()
        .get::<MeshMaterial3d<StandardMaterial>>(entity(app, path))
        .expect("material component exists")
        .0
        .clone()
}

fn mesh_handle(app: &App, path: &str) -> Handle<Mesh> {
    app.world()
        .get::<Mesh3d>(entity(app, path))
        .expect("mesh component exists")
        .0
        .clone()
}

fn apply_pending(app: &mut App) {
    let live = app
        .world_mut()
        .remove_non_send::<LiveStage>()
        .expect("live stage exists");
    let batch = live.drain_change_batch().expect("pending material edit");
    let mut map = app
        .world_mut()
        .remove_resource::<PrimEntities>()
        .expect("prim map exists");
    apply_change_batch(app.world_mut(), &live, &mut map, &batch);
    app.world_mut().insert_resource(map);
    app.world_mut().insert_non_send(live);
}

fn patch_only(app: &mut App, path: &str, property: &str) {
    let live = app
        .world_mut()
        .remove_non_send::<LiveStage>()
        .expect("live stage exists");
    let mut map = app
        .world_mut()
        .remove_resource::<PrimEntities>()
        .expect("prim map exists");
    let batch = StageChangeBatch {
        revision: LiveRevision(100),
        changes: vec![StageChange {
            resynced: Vec::new(),
            changed_info: vec![format!("{path}.{property}")],
        }],
    };
    apply_change_batch(app.world_mut(), &live, &mut map, &batch);
    app.world_mut().insert_resource(map);
    app.world_mut().insert_non_send(live);
}

fn set_material_color(app: &mut App, material: &str, color: [f32; 3]) {
    let live = app.world().get_non_send::<LiveStage>().expect("stage");
    let path = openusd::sdf::path(material).expect("material path");
    live.stage
        .prim(path)
        .attribute("inputs:diffuseColor")
        .set(Value::Vec3f(Vec3f::from(color)))
        .expect("material color authoring succeeds");
    apply_pending(app);
}

fn set_material_binding(app: &mut App, prim: &str, target: Option<&str>) {
    let live = app.world().get_non_send::<LiveStage>().expect("stage");
    let relationship = live
        .stage
        .prim(openusd::sdf::path(prim).expect("prim path"))
        .relationship("material:binding");
    match target {
        Some(target) => relationship
            .set_targets([openusd::sdf::path(target).expect("material path")])
            .expect("binding authoring succeeds"),
        None => relationship
            .set_targets(Vec::<openusd::sdf::Path>::new())
            .expect("binding removal succeeds"),
    };
    apply_pending(app);
}

fn set_network_shader_roughness(app: &mut App, roughness: f32) {
    let live = app.world().get_non_send::<LiveStage>().expect("stage");
    live.stage
        .prim(openusd::sdf::path(NETWORK_RED_SURFACE).unwrap())
        .attribute("inputs:roughness")
        .set(Value::Float(roughness))
        .expect("shader input authoring succeeds");
    apply_pending(app);
}

fn set_network_texture_path(app: &mut App, path: &str) {
    let live = app.world().get_non_send::<LiveStage>().expect("stage");
    live.stage
        .prim(openusd::sdf::path(NETWORK_RED_ALBEDO).unwrap())
        .attribute("inputs:file")
        .set(Value::AssetPath(path.into()))
        .expect("texture path authoring succeeds");
    apply_pending(app);
}

#[test]
fn material_patch_keeps_unrelated_edits_sparse_and_propagates_shared_inputs() {
    let mut app = build_app();
    let red_before = material_handle(&app, RED_BOX);
    let red_mesh = mesh_handle(&app, RED_BOX);
    let green_mesh = mesh_handle(&app, "/World/GreenBall");
    let initial_cache = app
        .world()
        .resource::<crate::route::material::UsdMaterialCache>()
        .stats();

    patch_only(&mut app, RED_BOX, "xformOp:translate");
    patch_only(&mut app, RED_BOX, "size");
    assert_eq!(material_handle(&app, RED_BOX), red_before);
    assert_eq!(mesh_handle(&app, RED_BOX), red_mesh);
    assert_eq!(
        app.world()
            .resource::<crate::route::material::UsdMaterialCache>()
            .stats(),
        initial_cache,
        "transform and geometry edits must not read material descriptors"
    );

    let green_before = material_handle(&app, "/World/GreenBall");
    set_material_color(&mut app, RED, [0.2, 0.3, 0.9]);
    assert_ne!(material_handle(&app, RED_BOX), red_before);
    assert_eq!(material_handle(&app, "/World/GreenBall"), green_before);
    assert_eq!(mesh_handle(&app, RED_BOX), red_mesh);
    assert_eq!(mesh_handle(&app, "/World/GreenBall"), green_mesh);

    let live = app.world().get_non_send::<LiveStage>().expect("stage");
    live.stage
        .prim(openusd::sdf::path(RED_BOX).unwrap())
        .relationship("material:binding")
        .set_targets([openusd::sdf::path(GREEN).unwrap()])
        .expect("binding authoring succeeds");
    apply_pending(&mut app);
    assert_eq!(
        material_handle(&app, RED_BOX),
        material_handle(&app, "/World/GreenBall"),
        "binding edit must switch to the existing shared material"
    );

    let before_unrelated = material_handle(&app, RED_BOX);
    set_material_color(&mut app, "/World/Materials/EmissiveBlue", [0.9, 0.8, 0.1]);
    assert_eq!(material_handle(&app, RED_BOX), before_unrelated);
    assert_eq!(mesh_handle(&app, RED_BOX), red_mesh);
    assert_eq!(mesh_handle(&app, "/World/GreenBall"), green_mesh);

    let diagnostics = *app.world().resource::<MaterialRouteDiagnostics>();
    assert!(diagnostics.matches > 0);
    assert!(diagnostics.projects > 0);
    assert!(diagnostics.patches >= 4);
    assert!(diagnostics.descriptor_reads >= 7);
}

#[test]
fn binding_removal_uses_shared_fallback_and_stops_old_material_fanout() {
    let mut app = build_app();
    let bound = material_handle(&app, RED_BOX);

    set_material_binding(&mut app, RED_BOX, None);
    let fallback = material_handle(&app, RED_BOX);
    assert_ne!(
        fallback, bound,
        "removing a binding must replace stale material"
    );

    set_material_binding(&mut app, RED_BOX, Some("/World/Materials/Missing"));
    assert_eq!(
        material_handle(&app, RED_BOX),
        fallback,
        "an unresolved binding must preserve the shared fallback"
    );

    set_material_color(&mut app, RED, [0.9, 0.2, 0.8]);
    assert_eq!(
        material_handle(&app, RED_BOX),
        fallback,
        "the former material must not reproject an unbound consumer"
    );

    set_material_binding(&mut app, RED_BOX, Some(RED));
    let rebound = material_handle(&app, RED_BOX);
    assert_ne!(
        rebound, fallback,
        "a valid rebind must restore a real material"
    );
    assert_ne!(
        rebound, bound,
        "the edited material must use a new descriptor"
    );
}

#[test]
fn shader_input_patch_reaches_only_its_material_consumers() {
    let mut app = build_app();
    let red_before = material_handle(&app, RED_BOX);
    let green_before = material_handle(&app, "/World/GreenBall");

    patch_only(&mut app, "/World/Materials/Red/Surface", "inputs:file");

    assert_eq!(material_handle(&app, RED_BOX), red_before);
    assert_eq!(material_handle(&app, "/World/GreenBall"), green_before);
    let diagnostics = *app.world().resource::<MaterialRouteDiagnostics>();
    assert!(diagnostics.patches > 0);
}

#[test]
fn external_shader_and_texture_edits_follow_real_network_dependencies() {
    let mut app = build_app_for("materials_network.usda");
    let red_before = material_handle(&app, NETWORK_RED_BOX);
    let red_ball_before = material_handle(&app, NETWORK_RED_BALL);
    let green_before = material_handle(&app, NETWORK_GREEN);

    set_network_shader_roughness(&mut app, 0.15);
    let red_after_shader = material_handle(&app, NETWORK_RED_BOX);
    let red_ball_after_shader = material_handle(&app, NETWORK_RED_BALL);
    assert_ne!(red_after_shader, red_before);
    assert_ne!(red_ball_after_shader, red_ball_before);
    assert_eq!(material_handle(&app, NETWORK_GREEN), green_before);

    app.world_mut()
        .resource_mut::<crate::route::material::UsdMaterialCache>()
        .reset_stats();
    app.world_mut()
        .resource_mut::<crate::route::material::UsdTextureCache>()
        .reset_stats();
    set_network_texture_path(
        &mut app,
        "assets/external/franka/panda/DetailedProps/Materials/Textures/Logo_Textures_Albedo.png",
    );

    assert_ne!(material_handle(&app, NETWORK_RED_BOX), red_after_shader);
    assert_ne!(
        material_handle(&app, NETWORK_RED_BALL),
        red_ball_after_shader
    );
    assert_eq!(material_handle(&app, NETWORK_GREEN), green_before);
    let material_stats = app
        .world()
        .resource::<crate::route::material::UsdMaterialCache>()
        .stats();
    let texture_stats = app
        .world()
        .resource::<crate::route::material::UsdTextureCache>()
        .stats();
    assert_eq!(material_stats.lookups, 2);
    assert_eq!(material_stats.misses, 1);
    assert_eq!(material_stats.hits, 1);
    assert_eq!(texture_stats.lookups, 1);
    assert_eq!(texture_stats.misses, 1);
    assert_eq!(texture_stats.hits, 0);
    assert_eq!(texture_stats.decode_calls, 1);
}

#[test]
fn shared_material_edit_cleans_retired_asset_once_after_fanout() {
    let mut app = build_app_for("materials_network.usda");
    assert_eq!(
        app.world().resource::<Assets<StandardMaterial>>().len(),
        3,
        "bound shapes must not leave placeholder StandardMaterials behind"
    );
    app.world_mut()
        .resource_mut::<crate::route::material::UsdMaterialCache>()
        .reset_stats();

    set_network_shader_roughness(&mut app, 0.2);

    let stats = app
        .world()
        .resource::<crate::route::material::UsdMaterialCache>()
        .stats();
    assert_eq!(
        stats.lookups, 2,
        "both shared consumers should be projected"
    );
    assert_eq!(stats.retired_assets, 1);
    assert_eq!(stats.cleaned_assets, 1);
    assert_eq!(stats.cleanup_passes, 1);
    assert_eq!(stats.cleanup_entities_scanned, 4);
    assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 3);
}

#[test]
fn repeated_material_edits_keep_standard_material_assets_bounded() {
    let mut app = build_app();
    let initial_assets = app.world().resource::<Assets<StandardMaterial>>().len();

    for i in 0..32 {
        let n = i as f32 / 32.0;
        set_material_color(&mut app, RED, [n, 1.0 - n, 0.25]);
    }

    let assets = app.world().resource::<Assets<StandardMaterial>>();
    let stats = app
        .world()
        .resource::<crate::route::material::UsdMaterialCache>()
        .stats();
    assert_eq!(assets.len(), initial_assets);
    assert_eq!(stats.retired_assets, stats.cleaned_assets);
    assert!(stats.cleaned_assets >= 32);
}
