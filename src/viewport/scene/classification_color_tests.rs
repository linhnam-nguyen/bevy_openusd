use super::*;

use bevy::pbr::MeshMaterial3d;
use usd_bevy::UsdPrimRef;
use viewport_protocol::{ClassificationColorEntry, SceneAnchor};

use crate::viewport::api::SceneAnchorIndex;

#[test]
fn color_plan_rebinds_shared_material_and_disable_restores_authored_route() {
    let anchor = SceneAnchor::active_session("/World/Wall");
    let authored = Handle::default();
    let mut app = App::new();
    app.init_resource::<Assets<StandardMaterial>>()
        .init_resource::<ClassificationColorPlan>()
        .init_resource::<ClassificationColorDiagnostics>()
        .init_resource::<ClassificationColorMaterialCache>()
        .add_systems(Update, sync_classification_color_overrides);
    let entity = app
        .world_mut()
        .spawn((
            UsdPrimRef::new("/World/Wall"),
            Mesh3d(Handle::default()),
            MeshMaterial3d(authored.clone()),
        ))
        .id();
    app.insert_resource(SceneAnchorIndex::from_test_entity(anchor.clone(), entity));

    app.world_mut()
        .resource_mut::<ClassificationColorPlan>()
        .replace(
            1,
            vec![ClassificationColorEntry {
                anchor: anchor.clone(),
                color: ColorRgb8::new(0x12, 0x34, 0x56),
            }],
        );
    app.update();

    let applied = app
        .world()
        .get::<MeshMaterial3d<StandardMaterial>>(entity)
        .expect("mesh material remains on the projected entity")
        .0
        .clone();
    assert_ne!(applied, authored);
    assert!(
        app.world()
            .get::<ClassificationColorOverride>(entity)
            .is_some()
    );
    assert_eq!(
        app.world()
            .resource::<ClassificationColorMaterialCache>()
            .handles
            .len(),
        1
    );

    let rebuilds = app
        .world()
        .resource::<ClassificationColorDiagnostics>()
        .rebuilds;
    app.update();
    assert_eq!(
        app.world()
            .resource::<ClassificationColorDiagnostics>()
            .rebuilds,
        rebuilds
    );

    app.world_mut()
        .resource_mut::<ClassificationColorPlan>()
        .replace(2, Vec::new());
    app.update();
    assert_eq!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .expect("mesh remains projected")
            .0,
        authored
    );
    assert!(
        app.world()
            .get::<ClassificationColorOverride>(entity)
            .is_none()
    );
    assert!(
        app.world()
            .get::<ClassificationBaseMaterial>(entity)
            .is_none()
    );
}

#[test]
fn color_plan_updates_selection_base_without_overwriting_selection_material() {
    let anchor = SceneAnchor::active_session("/World/Door");
    let authored = Handle::default();
    let selected = Handle::default();
    let mut app = App::new();
    app.init_resource::<Assets<StandardMaterial>>()
        .init_resource::<ClassificationColorPlan>()
        .init_resource::<ClassificationColorDiagnostics>()
        .init_resource::<ClassificationColorMaterialCache>()
        .add_systems(Update, sync_classification_color_overrides);
    let entity = app
        .world_mut()
        .spawn((
            UsdPrimRef::new("/World/Door"),
            Mesh3d(Handle::default()),
            MeshMaterial3d(selected.clone()),
            SelectionBaseMaterial(authored.clone()),
        ))
        .id();
    app.insert_resource(SceneAnchorIndex::from_test_entity(anchor.clone(), entity));

    app.world_mut()
        .resource_mut::<ClassificationColorPlan>()
        .replace(
            1,
            vec![ClassificationColorEntry {
                anchor,
                color: ColorRgb8::new(0xAB, 0xCD, 0xEF),
            }],
        );
    app.update();
    assert_eq!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .expect("selected mesh material remains")
            .0,
        selected
    );
    assert_ne!(
        app.world()
            .get::<SelectionBaseMaterial>(entity)
            .expect("selection base remains the composition boundary")
            .0,
        authored
    );

    app.world_mut()
        .resource_mut::<ClassificationColorPlan>()
        .replace(2, Vec::new());
    app.update();
    assert_eq!(
        app.world()
            .get::<SelectionBaseMaterial>(entity)
            .expect("selection base remains after restore")
            .0,
        authored
    );
}

#[test]
fn path_only_classification_entries_rebind_all_instance_occurrences() {
    let path = "/World/Window/Frame";
    let anchor = SceneAnchor::active_session(path);
    let instance_anchor = SceneAnchor {
        session_id: None,
        prim_path: path.to_owned(),
        instance_context: Some("occurrence-1".to_owned()),
    };
    let authored = Handle::default();
    let mut app = App::new();
    app.init_resource::<Assets<StandardMaterial>>()
        .init_resource::<ClassificationColorPlan>()
        .init_resource::<ClassificationColorDiagnostics>()
        .init_resource::<ClassificationColorMaterialCache>()
        .add_systems(Update, sync_classification_color_overrides);
    let first = app
        .world_mut()
        .spawn((
            UsdPrimRef::new(path),
            Mesh3d(Handle::default()),
            MeshMaterial3d(authored.clone()),
        ))
        .id();
    let second = app
        .world_mut()
        .spawn((
            UsdPrimRef::new(path),
            Mesh3d(Handle::default()),
            MeshMaterial3d(authored.clone()),
        ))
        .id();
    app.insert_resource(SceneAnchorIndex::from_test_entities(vec![
        (anchor.clone(), first),
        (instance_anchor, second),
    ]));

    app.world_mut()
        .resource_mut::<ClassificationColorPlan>()
        .replace(
            1,
            vec![ClassificationColorEntry {
                anchor,
                color: ColorRgb8::new(0x12, 0x34, 0x56),
            }],
        );
    app.update();

    let first_material = app
        .world()
        .get::<MeshMaterial3d<StandardMaterial>>(first)
        .expect("first instance mesh material")
        .0
        .clone();
    let second_material = app
        .world()
        .get::<MeshMaterial3d<StandardMaterial>>(second)
        .expect("second instance mesh material")
        .0
        .clone();
    assert_ne!(first_material, authored);
    assert_eq!(first_material, second_material);

    app.world_mut()
        .resource_mut::<ClassificationColorPlan>()
        .replace(2, Vec::new());
    app.update();
    assert_eq!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(first)
            .expect("first instance material after restore")
            .0,
        authored
    );
    assert_eq!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(second)
            .expect("second instance material after restore")
            .0,
        authored
    );
}
