use bevy::asset::{Assets, RenderAssetUsages};
use bevy::mesh::{Mesh, PrimitiveTopology};
use bevy::prelude::*;
use std::collections::HashMap;

use super::super::ProjectionSeed;
use super::super::cache::{MAX_INTERNED, ProjectionCache, remember_source_mesh};
use super::super::cache_key::source_mesh_key;
use super::{MeshPatchAction, MeshRoute, UsdDisplayName, UsdLocalExtent, mesh_patch_action};
use crate::read::geom::{Interpolation, MeshPrimvar, read_mesh};
use crate::route::{PrimRoute, RouteCtx};
use crate::snippet::UsdSnippet;
use crate::{
    USDHUB_HIERARCHY_ROLE_METADATA, USDHUB_TRANSPARENT_SOURCE_ROLE, UsdTransparentHierarchyNode,
};

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

#[test]
fn repeated_source_content_reuses_mesh_before_conversion() {
    let (stage, path) = mesh_stage();
    let ctx = RouteCtx::new(&stage, &path);
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    world.init_resource::<ProjectionCache>();
    let first_entity = world.spawn_empty().id();
    let second_entity = world.spawn_empty().id();
    let route = MeshRoute;

    route.project(&ctx, &mut world, first_entity);
    route.project(&ctx, &mut world, second_entity);

    assert_eq!(
        world.get::<Mesh3d>(first_entity).expect("first mesh").0,
        world.get::<Mesh3d>(second_entity).expect("second mesh").0,
        "identical source content must reuse one mesh handle"
    );
    assert_eq!(
        world.get_resource::<Assets<Mesh>>().map(Assets::len),
        Some(1)
    );
    assert_eq!(world.resource::<ProjectionCache>().stats().hits, 1);
}

#[test]
fn persistent_mesh_seed_is_consumed_by_the_normal_route() {
    let (stage, path) = mesh_stage();
    let ctx = RouteCtx::new(&stage, &path);
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    world.init_resource::<ProjectionSeed>();
    let mut seeded = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    seeded.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
    );
    seeded.insert_indices(bevy::mesh::Indices::U32(vec![0, 1, 2]));
    let handle = world.resource_mut::<Assets<Mesh>>().add(seeded);
    world.resource_mut::<ProjectionSeed>().insert_mesh(
        path.as_str(),
        handle.clone(),
        Some(([0.0; 3], [2.0, 2.0, 0.0])),
    );
    let entity = world.spawn_empty().id();

    MeshRoute.project(&ctx, &mut world, entity);

    assert_eq!(world.get::<Mesh3d>(entity).expect("seeded mesh").0, handle);
    assert_eq!(world.resource::<ProjectionSeed>().pending_meshes(), 0);
    assert_eq!(world.resource::<Assets<Mesh>>().len(), 1);
    assert_eq!(
        world.get::<UsdLocalExtent>(entity),
        Some(&UsdLocalExtent {
            min: [0.0, 0.0, 0.0],
            max: [2.0, 2.0, 0.0],
        })
    );
}

#[test]
fn hierarchy_metadata_route_projects_display_name_and_explicit_source_role() -> anyhow::Result<()> {
    let stage = openusd::usd::Stage::builder().in_memory("hierarchy-metadata.usda")?;
    let prim = stage
        .define_prim("/World/Source")?
        .set_type_name("Xform")?
        .set_metadata(
            "ui:displayName",
            openusd::sdf::Value::String("Friendly Source".to_owned()),
        )?
        .set_metadata(
            "customData",
            openusd::sdf::Value::Dictionary(HashMap::from([(
                USDHUB_HIERARCHY_ROLE_METADATA.to_owned(),
                openusd::sdf::Value::String(USDHUB_TRANSPARENT_SOURCE_ROLE.to_owned()),
            )])),
        )?;
    let path = openusd::sdf::path("/World/Source")?;
    let ctx = RouteCtx::new(&stage, &path);
    let mut world = World::new();
    let entity = world.spawn_empty().id();

    super::VisibilityRoute.project(&ctx, &mut world, entity);

    assert_eq!(
        world.get::<UsdDisplayName>(entity),
        Some(&UsdDisplayName("Friendly Source".to_owned()))
    );
    assert_eq!(
        world.get::<UsdTransparentHierarchyNode>(entity),
        Some(&UsdTransparentHierarchyNode)
    );
    let _ = prim;
    Ok(())
}

#[test]
fn source_mesh_key_changes_for_rendered_content_but_not_extent() {
    let (stage, path) = mesh_stage();
    let read = read_mesh(&stage, &path)
        .expect("mesh read succeeds")
        .expect("mesh exists");
    let key = source_mesh_key(&read);

    let mut points = read.clone();
    points.points[0][0] = 2.0;
    assert_ne!(source_mesh_key(&points), key);

    let mut topology = read.clone();
    topology.face_vertex_indices[0] = 2;
    assert_ne!(source_mesh_key(&topology), key);

    let mut uv = read.clone();
    uv.uvs = Some(MeshPrimvar {
        values: vec![[0.0, 0.0]; 3],
        interpolation: Interpolation::Vertex,
        indices: Vec::new(),
    });
    assert_ne!(source_mesh_key(&uv), key);

    let mut normal = read.clone();
    normal.normals = Some(MeshPrimvar {
        values: vec![[0.0, 0.0, 1.0]; 3],
        interpolation: Interpolation::Vertex,
        indices: Vec::new(),
    });
    assert_ne!(source_mesh_key(&normal), key);

    let mut color = read.clone();
    color.display_color = Some(MeshPrimvar {
        values: vec![[1.0, 0.0, 0.0]; 3],
        interpolation: Interpolation::Vertex,
        indices: Vec::new(),
    });
    assert_ne!(source_mesh_key(&color), key);

    let mut extent = read;
    extent.extent = Some([[-10.0, -10.0, -10.0], [10.0, 10.0, 10.0]]);
    assert_eq!(source_mesh_key(&extent), key);
}

#[test]
fn fallback_material_is_shared_across_meshes() {
    let (stage, path) = mesh_stage();
    let ctx = RouteCtx::new(&stage, &path);
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    let material_assets_before = world.resource::<Assets<StandardMaterial>>().len();
    let route = MeshRoute;
    let mut first_handle = None;
    let mut second_handle = None;
    for index in 0..2_400 {
        let entity = world.spawn_empty().id();
        route.project(&ctx, &mut world, entity);
        let handle = world
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .expect("fallback material attached")
            .0
            .clone();
        match index {
            0 => first_handle = Some(handle),
            1 => second_handle = Some(handle),
            _ => {}
        }
    }
    let material_assets_after = world.resource::<Assets<StandardMaterial>>().len();
    let first_handle = first_handle.expect("first fallback material");
    let second_handle = second_handle.expect("second fallback material");

    assert_eq!(
        first_handle, second_handle,
        "fallback material handle is shared"
    );
    assert_eq!(
        world
            .get_resource::<Assets<StandardMaterial>>()
            .map(Assets::len),
        Some(1)
    );
    assert_eq!(material_assets_before, 0);
    assert_eq!(material_assets_after, 1);
    let artifact = serde_json::json!({
        "schema": "usdhub.m5.c3.fallback-material.v1",
        "checkpoint": "M5-C3+",
        "projected_mesh_prims": 2_400,
        "standard_material_assets_before": material_assets_before,
        "standard_material_assets_after": material_assets_after,
        "shared_fallback_material": true,
    });
    let artifact_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/m5-c3-fallback-material.json");
    std::fs::write(
        &artifact_path,
        serde_json::to_vec_pretty(&artifact).expect("fallback artifact serializes"),
    )
    .expect("fallback artifact writes");
}

#[test]
fn source_cache_is_bounded_for_deforming_geometry_versions() {
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.insert_resource(ProjectionCache::default());

    for version in 0..=MAX_INTERNED {
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
        remember_source_mesh(&mut world, version as u64, handle);
    }

    let cache = world.resource::<ProjectionCache>();
    assert!(cache.source_len() <= MAX_INTERNED);
    assert!(cache.stats().evictions > 0);
}
