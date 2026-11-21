use bevy::asset::{Assets, RenderAssetUsages};
use bevy::mesh::{Mesh, PrimitiveTopology};
use bevy::prelude::*;
use std::collections::HashMap;

use super::super::ProjectionSeed;
use super::super::cache::{
    MAX_SOURCE_MESHES, ProjectionCache, lookup_source_mesh, remember_source_mesh,
};
use super::super::cache_key::{MeshSourceKey, source_mesh_key};
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
    let stage = openusd::usd::Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .in_memory("hierarchy-metadata.usda")?;
    let prim = stage
        .define_prim("/World/Source")?
        .set_type_name("Xform")?
        .set_metadata(
            "ui:displayName",
            openusd::vt::Value::String("Friendly Source".to_owned()),
        )?
        .set_metadata(
            "customData",
            openusd::vt::Value::Dictionary(openusd::vt::Dictionary::from([(
                USDHUB_HIERARCHY_ROLE_METADATA.to_owned(),
                openusd::vt::Value::String(USDHUB_TRANSPARENT_SOURCE_ROLE.to_owned()),
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
fn hierarchy_metadata_legacy_opinions_are_indexed_once_per_stage() -> anyhow::Result<()> {
    let stage = openusd::usd::Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .in_memory("hierarchy-index.usda")?;
    stage
        .define_prim("/World/Source")?
        .set_type_name("Xform")?
        .set_metadata(
            "ui:displayName",
            openusd::vt::Value::String("Indexed Source".to_owned()),
        )?;
    let mut world = World::new();

    super::prepare_hierarchy_metadata(&stage, &mut world);
    let index = world.resource::<super::hierarchy::HierarchyMetadataIndex>();
    assert_eq!(index.display_name("/World/Source"), Some("Indexed Source"));

    let path = openusd::sdf::path("/World/Source")?;
    let ctx = RouteCtx::new(&stage, &path);
    let entity = world.spawn_empty().id();
    super::VisibilityRoute.project(&ctx, &mut world, entity);
    assert_eq!(
        world.get::<UsdDisplayName>(entity),
        Some(&UsdDisplayName("Indexed Source".to_owned()))
    );
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

    for version in 0..=MAX_SOURCE_MESHES {
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
        remember_source_mesh(
            &mut world,
            MeshSourceKey(Box::new(version.to_le_bytes())),
            handle,
        );
    }

    let cache = world.resource::<ProjectionCache>();
    assert_eq!(cache.source_len(), MAX_SOURCE_MESHES);
    assert_eq!(cache.stats().evictions, 0);
    let first = MeshSourceKey(Box::new(0usize.to_le_bytes()));
    let overflow = MeshSourceKey(Box::new(MAX_SOURCE_MESHES.to_le_bytes()));
    assert!(lookup_source_mesh(&mut world, &first).is_some());
    assert!(lookup_source_mesh(&mut world, &overflow).is_none());
}

fn projected_fixture_app(fixture: &str) -> App {
    let stage = usd_support::open_fixture_stage(fixture);
    let mut app = App::new();
    app.add_plugins(crate::UsdPlugin)
        .add_plugins(crate::LiveStagePlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut()
        .insert_non_send(crate::LiveStage::new(stage));
    app.update();
    app
}

#[test]
fn projects_basis_curves_and_points_with_current_topology() {
    let mut app = projected_fixture_app("curves_points.usda");
    let world = app.world_mut();
    let mut by_path = HashMap::new();
    for (prim, mesh) in world.query::<(&crate::UsdPrimRef, &Mesh3d)>().iter(world) {
        by_path.insert(prim.path.clone(), mesh.0.clone());
    }
    for expected in ["/World/Polyline", "/World/Curve", "/World/Cloud"] {
        assert!(
            by_path.contains_key(expected),
            "missing Mesh3d on {expected}"
        );
    }

    let meshes = app.world().resource::<Assets<Mesh>>();
    let polyline = meshes.get(&by_path["/World/Polyline"]).unwrap();
    assert_eq!(polyline.primitive_topology(), PrimitiveTopology::LineList);
    assert_eq!(polyline.count_vertices(), 5);

    let curve = meshes.get(&by_path["/World/Curve"]).unwrap();
    assert_eq!(curve.primitive_topology(), PrimitiveTopology::LineList);
    assert_eq!(curve.count_vertices(), 17);

    let cloud = meshes.get(&by_path["/World/Cloud"]).unwrap();
    assert_eq!(cloud.primitive_topology(), PrimitiveTopology::PointList);
    assert_eq!(cloud.count_vertices(), 6);
}

#[test]
fn projects_primitives_with_meshes() {
    let mut app = projected_fixture_app("primitives.usda");
    let world = app.world_mut();
    let mut by_path = HashMap::new();
    for (prim, mesh) in world.query::<(&crate::UsdPrimRef, &Mesh3d)>().iter(world) {
        by_path.insert(prim.path.clone(), mesh.0.clone());
    }
    for expected in [
        "/World/Box",
        "/World/Ball",
        "/World/Pipe",
        "/World/Cap",
        "/World/Tri",
    ] {
        assert!(
            by_path.contains_key(expected),
            "missing Mesh3d on {expected}"
        );
    }
    assert!(!by_path.contains_key("/World"));

    let meshes = app.world().resource::<Assets<Mesh>>();
    let tri = meshes.get(&by_path["/World/Tri"]).unwrap();
    assert_eq!(tri.count_vertices(), 3);
}

#[test]
fn reads_subdivision_scheme_and_classifies_meshes() {
    use crate::read::geom::SubdivScheme;
    use crate::route::profile::GeometrySubdivisionClass;

    let stage = usd_support::open_fixture_stage("subdivision.usda");
    let cc = read_mesh(&stage, &openusd::sdf::path("/World/CatmullClark").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(cc.subdivision_scheme, SubdivScheme::CatmullClark);
    assert_eq!(
        GeometrySubdivisionClass::from(cc.subdivision_scheme),
        GeometrySubdivisionClass::CatmullClark
    );

    let lp = read_mesh(&stage, &openusd::sdf::path("/World/LoopTri").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(lp.subdivision_scheme, SubdivScheme::Loop);
    assert_eq!(
        GeometrySubdivisionClass::from(lp.subdivision_scheme),
        GeometrySubdivisionClass::Loop
    );

    let flat = read_mesh(&stage, &openusd::sdf::path("/World/Flat").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(flat.subdivision_scheme, SubdivScheme::None);

    let un = read_mesh(&stage, &openusd::sdf::path("/World/Unauthored").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(un.subdivision_scheme, SubdivScheme::None);
}

#[test]
fn reads_custom_attrs_and_filters_schema_ones() {
    use crate::read::geom::{CustomAttrValue, read_custom_attrs};

    let stage = usd_support::open_fixture_stage("custom_attrs.usda");
    let plain = read_custom_attrs(&stage, &openusd::sdf::path("/World/Plain").unwrap()).unwrap();
    assert!(plain.is_empty());

    let robot = read_custom_attrs(&stage, &openusd::sdf::path("/World/Robot").unwrap()).unwrap();
    assert_eq!(robot.len(), 5);
    let by_name: HashMap<_, _> = robot.into_iter().collect();
    assert!(!by_name.contains_key("xformOp:translate"));
    assert_eq!(
        by_name.get("userProperties:max_speed"),
        Some(&CustomAttrValue::Float(2.5))
    );
    assert_eq!(
        by_name.get("userProperties:priority"),
        Some(&CustomAttrValue::Int(7))
    );
    assert_eq!(
        by_name.get("userProperties:name"),
        Some(&CustomAttrValue::String("cart_01".to_string()))
    );
    assert_eq!(
        by_name.get("userProperties:active"),
        Some(&CustomAttrValue::Bool(true))
    );
    match by_name.get("arena:tint") {
        Some(CustomAttrValue::Vec3f(rgb)) => {
            assert!((rgb[0] - 0.85).abs() < 1e-4);
            assert!((rgb[1] - 0.55).abs() < 1e-4);
            assert!((rgb[2] - 0.1).abs() < 1e-4);
        }
        other => panic!("arena:tint should decode as Vec3f, got {other:?}"),
    }
}

#[test]
fn attaches_audio_and_procedural_components_to_right_prims() {
    let mut app = projected_fixture_app("media_proc.usda");
    let world = app.world_mut();

    let mut audio = Vec::new();
    for (prim, comp) in world
        .query::<(&crate::UsdPrimRef, &crate::UsdSpatialAudio)>()
        .iter(world)
    {
        audio.push((prim.path.clone(), comp.clone()));
    }
    audio.sort_by(|a, b| a.0.cmp(&b.0));
    let audio_paths: Vec<_> = audio.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(audio_paths, vec!["/World/Bell", "/World/Click"]);
    assert_eq!(audio[0].1.file, "sounds/bell.wav");
    assert_eq!(audio[0].1.aural_mode.as_deref(), Some("spatial"));
    assert_eq!(audio[0].1.playback_mode.as_deref(), Some("loopFromStart"));
    assert!((audio[0].1.gain - 0.8).abs() < 1e-6);

    let proc_entries: Vec<_> = world
        .query::<(&crate::UsdPrimRef, &crate::UsdProcedural)>()
        .iter(world)
        .map(|(p, c)| (p.path.clone(), c.system.clone()))
        .collect();
    assert_eq!(
        proc_entries,
        vec![("/World/Forest".to_string(), Some("houdini".to_string()))]
    );
}

#[test]
fn proxy_purpose_collider_survives_and_carries_physics() {
    let mut app = projected_fixture_app("physics_proxy.usda");
    let world = app.world_mut();
    let entity_of = |world: &mut World, target: &str| -> Entity {
        world
            .query::<(Entity, &crate::UsdPrimRef)>()
            .iter(world)
            .find(|(_, p)| p.path == target)
            .map(|(e, _)| e)
            .unwrap_or_else(|| panic!("missing {target}"))
    };

    let render_e = entity_of(world, "/World/Robot/RenderMesh");
    let proxy_e = entity_of(world, "/World/Robot/ProxyCollider");
    let guide_e = entity_of(world, "/World/Robot/DebugMarker");

    assert_eq!(
        world
            .get::<crate::UsdPurpose>(render_e)
            .map(|p| p.0.as_str()),
        Some("render")
    );
    assert_eq!(
        world
            .get::<crate::UsdPurpose>(proxy_e)
            .map(|p| p.0.as_str()),
        Some("proxy")
    );
    assert_eq!(
        world
            .get::<crate::UsdPurpose>(guide_e)
            .map(|p| p.0.as_str()),
        Some("guide")
    );
    assert!(world.get::<crate::UsdRigidBody>(proxy_e).is_some());
    assert!(world.get::<crate::UsdCollider>(proxy_e).is_some());
    // DisplayPurposes::default() shows proxy (Inherited) and hides render and guide.
    assert_eq!(world.get::<Visibility>(render_e), Some(&Visibility::Hidden));
    assert_eq!(
        world.get::<Visibility>(proxy_e),
        Some(&Visibility::Inherited)
    );
    assert_eq!(world.get::<Visibility>(guide_e), Some(&Visibility::Hidden));
}

#[test]
fn projects_supported_physics_marker_components() {
    let mut app = projected_fixture_app("physics.usda");
    let world = app.world_mut();
    let entity_of = |world: &mut World, target: &str| -> Entity {
        world
            .query::<(Entity, &crate::UsdPrimRef)>()
            .iter(world)
            .find(|(_, p)| p.path == target)
            .map(|(e, _)| e)
            .unwrap_or_else(|| panic!("missing {target}"))
    };

    let base_e = entity_of(world, "/World/Base");
    let arm_e = entity_of(world, "/World/Arm");
    let hinge_e = entity_of(world, "/World/Hinge");

    assert!(world.get::<crate::UsdRigidBody>(base_e).is_some());
    assert!(world.get::<crate::UsdCollider>(base_e).is_some());
    let base_mass = world.get::<crate::UsdMass>(base_e).expect("Base UsdMass");
    assert!((base_mass.mass.expect("mass") - 2.5).abs() < 1e-5);

    assert!(world.get::<crate::UsdRigidBody>(arm_e).is_some());
    assert!(world.get::<crate::UsdCollider>(arm_e).is_some());
    assert!(world.get::<crate::UsdMass>(arm_e).is_none());

    assert!(world.get::<crate::UsdPhysicsJoint>(hinge_e).is_some());
    let joint = world
        .get::<crate::UsdJoint>(hinge_e)
        .expect("Hinge UsdJoint");
    assert_eq!(joint.kind, "revolute");
    assert_eq!(joint.body0.as_deref(), Some("/World/Base"));
    assert_eq!(joint.body1.as_deref(), Some("/World/Arm"));
    assert_eq!(joint.axis.as_deref(), Some("Z"));
    assert_eq!(joint.lower, Some(-45.0));
    assert_eq!(joint.upper, Some(45.0));
}

#[test]
fn projects_supported_render_settings_markers() {
    let mut app = projected_fixture_app("render_settings.usda");
    let world = app.world_mut();
    let settings: Vec<_> = world
        .query::<(
            &crate::UsdPrimRef,
            &crate::route::coverage::UsdRenderSettings,
        )>()
        .iter(world)
        .map(|(p, s)| (p.path.clone(), s.clone()))
        .collect();
    assert_eq!(settings.len(), 1);
    assert_eq!(settings[0].0, "/Render/Primary");
    assert_eq!(
        settings[0].1.included_purposes,
        vec!["default".to_string(), "render".to_string()]
    );
}
