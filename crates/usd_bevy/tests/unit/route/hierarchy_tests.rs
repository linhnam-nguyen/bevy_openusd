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
