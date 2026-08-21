use bevy::asset::Assets;
use bevy::prelude::*;

use super::{MeshPatchAction, MeshRoute, UsdLocalExtent, mesh_patch_action};
use crate::route::{PrimRoute, RouteCtx};
use crate::snippet::UsdSnippet;

fn mesh_stage() -> (openusd::usd::Stage, openusd::sdf::Path) {
    let stage = UsdSnippet::new(
        r#"#usda 1.0
def Xform "World"
{
    def Mesh "Triangle"
    {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        float3[] extent = [(0, 0, 0), (1, 1, 0)]
    }
}
"#,
    )
    .open_stage()
    .expect("mesh test stage opens");
    let path = openusd::sdf::path("/World/Triangle").expect("mesh path is valid");
    (stage, path)
}

#[test]
fn mesh_patch_ignores_non_geometry_changes() {
    for properties in [
        ["xformOp:translate"].as_slice(),
        ["visibility"].as_slice(),
        ["material:binding"].as_slice(),
        ["kind"].as_slice(),
        ["customData:author"].as_slice(),
        ["bevy:selectionTag"].as_slice(),
    ] {
        assert_eq!(mesh_patch_action(properties), MeshPatchAction::Ignore);
    }
}

#[test]
fn mesh_patch_rebuilds_for_geometry_and_unknown_changes() {
    for properties in [
        ["points"].as_slice(),
        ["faceVertexIndices"].as_slice(),
        ["primvars:st:indices"].as_slice(),
        ["futureGeometryAttribute"].as_slice(),
        [].as_slice(),
    ] {
        assert_eq!(mesh_patch_action(properties), MeshPatchAction::Rebuild);
    }
}

#[test]
fn mesh_patch_updates_extent_without_replacing_mesh() {
    let (stage, path) = mesh_stage();
    let ctx = RouteCtx::new(&stage, &path);
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    let entity = world.spawn_empty().id();
    let route = MeshRoute;

    route.project(&ctx, &mut world, entity);
    let before = world
        .get::<Mesh3d>(entity)
        .expect("mesh was projected")
        .0
        .clone();

    route.patch(&ctx, &mut world, entity, &["extent"]);

    let after = world
        .get::<Mesh3d>(entity)
        .expect("mesh remains attached")
        .0
        .clone();
    assert_eq!(before, after, "extent-only patch must not rebuild the mesh");
    assert_eq!(
        world.get_resource::<Assets<Mesh>>().map(Assets::len),
        Some(1)
    );
    assert_eq!(
        world.get::<UsdLocalExtent>(entity),
        Some(&UsdLocalExtent {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 0.0],
        })
    );
}

#[test]
fn mesh_patch_rebuilds_geometry_owned_mesh() {
    let (stage, path) = mesh_stage();
    let ctx = RouteCtx::new(&stage, &path);
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    let entity = world.spawn_empty().id();
    let route = MeshRoute;

    route.project(&ctx, &mut world, entity);
    let before = world
        .get::<Mesh3d>(entity)
        .expect("mesh was projected")
        .0
        .clone();

    route.patch(&ctx, &mut world, entity, &["points"]);

    let after = world
        .get::<Mesh3d>(entity)
        .expect("mesh was rebuilt")
        .0
        .clone();
    assert_ne!(before, after, "geometry patch must replace the mesh handle");
    assert_eq!(
        world.get_resource::<Assets<Mesh>>().map(Assets::len),
        Some(2)
    );
}
