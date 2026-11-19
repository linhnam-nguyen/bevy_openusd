use bevy::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{PrimRoute, RouteCtx, UsdDisplayName, VisibilityRoute};

#[test]
fn hierarchy_metadata_composed_reference_descendant_uses_composed_path() -> anyhow::Result<()> {
    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    let fixture_id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let fixture_root = std::env::temp_dir();
    let child_path: PathBuf = fixture_root.join(format!(
        "usd_bevy_c6_composed_child_{}_{}.usda",
        std::process::id(),
        fixture_id
    ));
    let parent_path: PathBuf = fixture_root.join(format!(
        "usd_bevy_c6_composed_parent_{}_{}.usda",
        std::process::id(),
        fixture_id
    ));
    let child_path_str = child_path.to_string_lossy().into_owned();
    let parent_path_str = parent_path.to_string_lossy().into_owned();

    let child = openusd::usd::Stage::builder().in_memory("c6-composed-child.usda")?;
    child
        .define_prim("/SceneRoot/Member_Kitchen")?
        .set_type_name("Xform")?
        .set_metadata(
            "ui:displayName",
            openusd::sdf::Value::String("Kitchen_set".to_owned()),
        )?;
    child.root_layer().export(&child_path_str)?;

    let parent = openusd::usd::Stage::builder().in_memory("c6-composed-parent.usda")?;
    parent
        .define_prim("/SceneRoot")?
        .set_type_name("Xform")?
        .set_metadata(
            "ui:displayName",
            openusd::sdf::Value::String("Pro3".to_owned()),
        )?;
    parent
        .define_prim("/SceneRoot/Member_Sc1")?
        .set_type_name("Xform")?
        .set_metadata(
            "references",
            openusd::sdf::Value::ReferenceListOp(openusd::sdf::ReferenceListOp::prepended([
                openusd::sdf::Reference {
                    asset_path: child_path_str,
                    prim_path: openusd::sdf::path("/SceneRoot")?,
                    ..Default::default()
                },
            ])),
        )?;
    parent.root_layer().export(&parent_path_str)?;

    let stage = openusd::usd::Stage::open(&parent_path_str)?;
    let composed_path = openusd::sdf::path("/SceneRoot/Member_Sc1/Member_Kitchen")?;
    assert!(stage.prim(composed_path.clone()).is_defined()?);

    let mut world = World::new();
    super::prepare_hierarchy_metadata(&stage, &mut world);
    let index = world.resource::<super::hierarchy::HierarchyMetadataIndex>();
    assert_eq!(
        index.display_name(composed_path.as_str()),
        Some("Kitchen_set")
    );

    let ctx = RouteCtx::new(&stage, &composed_path);
    let entity = world.spawn_empty().id();
    VisibilityRoute.project(&ctx, &mut world, entity);
    assert_eq!(
        world.get::<UsdDisplayName>(entity),
        Some(&UsdDisplayName("Kitchen_set".to_owned()))
    );
    assert_eq!(ctx.prim_str(), "/SceneRoot/Member_Sc1/Member_Kitchen");

    let _ = std::fs::remove_file(child_path);
    let _ = std::fs::remove_file(parent_path);
    Ok(())
}

#[test]
fn projects_three_prim_hierarchy() {
    let stage = usd_support::open_fixture_stage("hierarchy.usda");
    assert_eq!(
        stage.stage_metadata("defaultPrim").unwrap(),
        Some(openusd::sdf::Value::Token("World".into()))
    );
    assert!(!stage.layer_stack().is_empty());

    let mut app = App::new();
    app.add_plugins(crate::UsdPlugin)
        .add_plugins(crate::LiveStagePlugin)
        .init_resource::<Assets<bevy::mesh::Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut().insert_non_send(crate::LiveStage::new(stage));
    app.update();

    let world = app.world_mut();
    let mut by_path = std::collections::HashMap::new();
    for (entity, prim) in world.query::<(Entity, &crate::UsdPrimRef)>().iter(world) {
        if prim.path != "/" {
            by_path.insert(prim.path.clone(), entity);
        }
    }
    assert!(by_path.contains_key("/World"));
    assert!(by_path.contains_key("/World/ChildA"));
    assert!(by_path.contains_key("/World/ChildB"));
    assert_eq!(by_path.len(), 3);

    let world_children = app
        .world()
        .get::<Children>(by_path["/World"])
        .map(|c| c.len())
        .unwrap_or(0);
    assert_eq!(world_children, 2);
    assert_eq!(
        app.world()
            .get::<crate::UsdPrimRef>(by_path["/World/ChildA"])
            .map(|p| p.path.as_str()),
        Some("/World/ChildA")
    );
}

#[test]
fn attaches_display_name_component_and_omits_plain() {
    let stage = usd_support::open_fixture_stage("usdui.usda");
    let mut app = App::new();
    app.add_plugins(crate::UsdPlugin)
        .add_plugins(crate::LiveStagePlugin)
        .init_resource::<Assets<bevy::mesh::Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut().insert_non_send(crate::LiveStage::new(stage));
    app.update();

    let world = app.world_mut();
    let mut by_path = std::collections::HashMap::new();
    for (prim, dn) in world
        .query::<(&crate::UsdPrimRef, Option<&UsdDisplayName>)>()
        .iter(world)
    {
        by_path.insert(prim.path.clone(), dn.map(|d| d.0.clone()));
    }
    assert_eq!(
        by_path.get("/World").and_then(|o| o.as_deref()),
        Some("Top of the world")
    );
    assert_eq!(
        by_path
            .get("/World/robot_v3_baseLink_geom_0")
            .and_then(|o| o.as_deref()),
        Some("Main Body")
    );
    assert_eq!(by_path.get("/World/Plain").and_then(|o| o.as_deref()), None);
}

#[test]
fn applies_visibility_and_reads_kind() {
    let stage = usd_support::open_fixture_stage("visibility_kind.usda");
    assert_eq!(
        crate::read::geom::read_kind(&stage, &openusd::sdf::path("/World").unwrap())
            .unwrap()
            .as_deref(),
        Some("assembly")
    );
    assert_eq!(
        crate::read::geom::read_kind(&stage, &openusd::sdf::path("/World/Robot").unwrap())
            .unwrap()
            .as_deref(),
        Some("component")
    );
    assert!(
        crate::read::geom::read_kind(&stage, &openusd::sdf::path("/World/Visible").unwrap())
            .unwrap()
            .is_none()
    );

    let mut app = App::new();
    app.add_plugins(crate::UsdPlugin)
        .add_plugins(crate::LiveStagePlugin)
        .init_resource::<Assets<bevy::mesh::Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut().insert_non_send(crate::LiveStage::new(stage));
    app.update();

    let world = app.world_mut();
    let mut by_path = std::collections::HashMap::new();
    for (prim, vis) in world.query::<(&crate::UsdPrimRef, &Visibility)>().iter(world) {
        by_path.insert(prim.path.clone(), *vis);
    }
    assert_eq!(by_path["/World/Visible"], Visibility::Inherited);
    assert_eq!(by_path["/World/Hidden"], Visibility::Hidden);
    assert_eq!(by_path["/World/HiddenParent"], Visibility::Hidden);
    assert_eq!(
        by_path["/World/HiddenParent/InheritsHidden"],
        Visibility::Inherited
    );
}
