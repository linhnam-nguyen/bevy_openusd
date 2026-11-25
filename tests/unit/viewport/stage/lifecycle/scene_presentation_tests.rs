use super::*;
use crate::cache::contract::{
    CachedTransform, SCENE_CACHE_INDEX_SCHEMA_VERSION, SceneCacheAddress, SceneCacheBlobRef,
    SceneCacheEntry, SceneCacheOccurrence,
};
use usd_project::ScenePlacementTransform;

fn source_stage_for_cache(activation: &SceneCacheActivation) -> openusd::usd::Stage {
    use openusd::vt::Value;

    let stage = openusd::usd::Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .in_memory("cache-presentation-fixture.usda")
        .expect("create test Stage");
    for entry in &activation.index.entries {
        let (path, target) = match &entry.kind {
            SceneCacheEntryKind::OwnedPrim { prim_path } => (prim_path.clone(), None),
            SceneCacheEntryKind::ChildScene {
                scene_id,
                member_id,
            } => (
                crate::project::scene::authoring::scene_member_path(*member_id),
                Some(("scene", scene_id.to_string())),
            ),
            SceneCacheEntryKind::ChildModel {
                model_id,
                member_id,
            } => (
                crate::project::scene::authoring::scene_member_path(*member_id),
                Some(("model", model_id.to_string())),
            ),
        };
        let prim = stage
            .define_prim(path.as_str())
            .expect("define cached test prim")
            .set_type_name("Xform")
            .expect("type cached test prim");
        if let Some((kind, id)) = target {
            prim.set_metadata(
                "customData",
                Value::Dictionary(
                    [
                        ("usdhub:targetKind".into(), Value::String(kind.to_owned())),
                        ("usdhub:targetId".into(), Value::String(id)),
                    ]
                    .into_iter()
                    .collect(),
                ),
            )
            .expect("author typed member metadata");
        }
    }
    stage
}

fn publish_fixture_cache(world: &mut World, activation: &SceneCacheActivation) {
    let source = source_stage_for_cache(activation);
    publish_scene_cache_presentation(world, activation, None);
    publish_cached_hierarchy(world, activation, &source);
}

#[test]
fn child_flags_are_computed_in_one_parent_index_pass() {
    let mut app = App::new();
    app.init_resource::<CurrentHierarchyProjection>();
    let scene_id = usd_project::SceneId::new_v4();
    let root = SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::PrimPath("/SceneRoot/Root".into()),
        },
        parent: None,
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: false,
        bim_enabled: false,
        geometry: None,
        material: None,
        animation: None,
        semantic_key: None,
        kind: SceneCacheEntryKind::OwnedPrim {
            prim_path: "/SceneRoot/Root".into(),
        },
        content_hash: None,
    };
    let child = SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::PrimPath("/SceneRoot/Root/Child".into()),
        },
        parent: Some(0),
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: true,
        bim_enabled: false,
        geometry: Some(SceneCacheBlobRef {
            blob_id: usd_model::BlobId("a".repeat(64)),
            byte_size: 1,
        }),
        material: None,
        animation: None,
        semantic_key: None,
        kind: SceneCacheEntryKind::OwnedPrim {
            prim_path: "/SceneRoot/Root/Child".into(),
        },
        content_hash: Some(usd_model::HashDigest::from_hex(&"a".repeat(64)).unwrap()),
    };
    let mut descriptor = crate::cache::SceneCacheDescriptorV3::invalidated(
        scene_id,
        1,
        usd_model::HashDigest::from_hex(&"b".repeat(64)).unwrap(),
    );
    descriptor.state = crate::cache::contract::SceneCacheState::Ready;
    descriptor.prim_count = 2;
    descriptor.cacheable_count = 1;
    let activation = SceneCacheActivation {
        descriptor,
        index: crate::cache::contract::SceneCacheIndex {
            schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
            scene_id,
            generation: 1,
            entries: vec![root, child],
        },
    };
    let source = usd_bevy::UsdSnippet::new(
        r#"#usda 1.0
    def Xform "SceneRoot" {
        def Xform "Root" {
            custom string ui:displayName = "Primary pump"
            def Xform "Child" {}
        }
    }
    "#,
    )
    .open_stage()
    .expect("open named source fixture");
    source
        .prim("/SceneRoot/Root/Child")
        .expect("child prim")
        .set_metadata(
            "ui:displayName",
            openusd::vt::Value::String("Legacy valve".to_owned()),
        )
        .expect("author legacy display-name metadata");

    // Old cache records cannot prove authored names before the source opens.
    publish_scene_cache_presentation(app.world_mut(), &activation, None);
    assert!(
        app.world()
            .resource::<CurrentHierarchyProjection>()
            .snapshot()
            .nodes
            .is_empty()
    );
    publish_cached_hierarchy(app.world_mut(), &activation, &source);
    let cached = app
        .world()
        .resource::<CurrentHierarchyProjection>()
        .snapshot();
    assert!(cached.nodes.iter().any(|node| node.has_children));
    let cached_names = cached
        .nodes
        .iter()
        .map(|node| (node.breadcrumb.clone(), node.name.clone()))
        .collect::<HashMap<_, _>>();
    assert_eq!(
        cached_names.get("/SceneRoot/Root").map(String::as_str),
        Some("Primary pump")
    );
    assert_eq!(
        cached_names
            .get("/SceneRoot/Root/Child")
            .map(String::as_str),
        Some("Legacy valve")
    );

    // Exercise the real live metadata route and bounded index publication,
    // rather than manufacturing a second set of expected read-model rows.
    use crate::viewport::rendering::scene::{
        SceneAnchorIndex, refresh_scene_anchor_index, register_scene_index_observers,
    };
    use crate::viewport::stage::Spawned;
    use usd_bevy::{PrimRoute, RouteCtx};
    let mut live = App::new();
    live.add_plugins(MinimalPlugins)
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<CurrentHierarchyProjection>()
        .init_resource::<Spawned>()
        .add_systems(Update, refresh_scene_anchor_index);
    register_scene_index_observers(&mut live);
    let root = live
        .world_mut()
        .spawn(usd_bevy::UsdPrimRef::new("/SceneRoot/Root"))
        .id();
    let child = live
        .world_mut()
        .spawn((
            usd_bevy::UsdPrimRef::new("/SceneRoot/Root/Child"),
            bevy::ecs::hierarchy::ChildOf(root),
        ))
        .id();
    for (entity, path) in [(root, "/SceneRoot/Root"), (child, "/SceneRoot/Root/Child")] {
        let path = openusd::sdf::path(path).expect("valid prim path");
        usd_bevy::route::geom::VisibilityRoute.project(
            &RouteCtx::new(&source, &path),
            live.world_mut(),
            entity,
        );
    }
    live.update();
    live.update();
    let live_projection = live
        .world()
        .resource::<CurrentHierarchyProjection>()
        .snapshot();
    let live_names = live_projection
        .nodes
        .iter()
        .map(|node| (node.breadcrumb.clone(), node.name.clone()))
        .collect::<HashMap<_, _>>();
    assert_eq!(
        cached_names, live_names,
        "cache and live names differ for the same composed Stage"
    );

    // A missing canonical prim is unknown metadata, not a verified absence.
    let missing = openusd::usd::Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .in_memory("missing-cache-prim.usda")
        .expect("create missing-source fixture");
    publish_cached_hierarchy(app.world_mut(), &activation, &missing);
    assert!(
        app.world()
            .resource::<CurrentHierarchyProjection>()
            .snapshot()
            .nodes
            .is_empty()
    );
}

#[test]
fn cached_presentation_ignores_semantic_key_and_shares_stage_presentation_naming() {
    let scene_id = usd_project::SceneId::new_v4();
    let child_scene_id = usd_project::SceneId::new_v4();
    let child_model_id = usd_project::ModelId::new_v4();
    let scene_member_id = usd_project::SceneMemberId::new_v4();
    let model_member_id = usd_project::SceneMemberId::new_v4();

    let root = SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::PrimPath("/SceneRoot".into()),
        },
        parent: None,
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: false,
        bim_enabled: false,
        geometry: None,
        material: None,
        animation: None,
        semantic_key: Some("semantic-id-must-not-display".into()),
        kind: SceneCacheEntryKind::OwnedPrim {
            prim_path: "/SceneRoot".into(),
        },
        content_hash: None,
    };
    let child_scene = SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::Member(scene_member_id),
        },
        parent: None,
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: false,
        bim_enabled: false,
        geometry: None,
        material: None,
        animation: None,
        semantic_key: Some("member-scene-semantic-key".into()),
        kind: SceneCacheEntryKind::ChildScene {
            scene_id: child_scene_id,
            member_id: scene_member_id,
        },
        content_hash: None,
    };
    let child_model = SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::Member(model_member_id),
        },
        parent: None,
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: false,
        bim_enabled: false,
        geometry: None,
        material: None,
        animation: None,
        semantic_key: Some("member-model-semantic-key".into()),
        kind: SceneCacheEntryKind::ChildModel {
            model_id: child_model_id,
            member_id: model_member_id,
        },
        content_hash: None,
    };
    let mut descriptor = crate::cache::SceneCacheDescriptorV3::invalidated(
        scene_id,
        2,
        usd_model::HashDigest::from_hex(&"c".repeat(64)).unwrap(),
    );
    descriptor.state = crate::cache::contract::SceneCacheState::Ready;
    descriptor.prim_count = 3;
    let activation = SceneCacheActivation {
        descriptor,
        index: crate::cache::contract::SceneCacheIndex {
            schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
            scene_id,
            generation: 2,
            entries: vec![root, child_scene, child_model],
        },
    };

    // Project activation resolves Project root name and explicit Scene/Model names.
    let mut project_app = App::new();
    project_app.init_resource::<CurrentHierarchyProjection>();
    let mut target_names = std::collections::HashMap::new();
    target_names.insert(("scene".to_owned(), child_scene_id.to_string()), "Sc1".to_owned());
    target_names.insert(
        ("model".to_owned(), child_model_id.to_string()),
        "Turbine".to_owned(),
    );
    project_app.insert_resource(StagePresentationContext {
        root_path: Some("/SceneRoot".to_owned()),
        root_name: Some("Pro3".to_owned()),
        target_names: target_names.clone(),
    });
    publish_fixture_cache(project_app.world_mut(), &activation);
    let projection = project_app.world().resource::<CurrentHierarchyProjection>();
    let nodes = &projection.snapshot().nodes;
    let root_node = nodes
        .iter()
        .find(|node| node.breadcrumb == "/SceneRoot")
        .expect("root node is published");
    assert_eq!(root_node.name, "Pro3");
    assert_eq!(
        root_node.anchor.as_ref().map(|anchor| anchor.prim_path.as_str()),
        Some("/SceneRoot")
    );
    assert!(root_node.has_children);
    assert!(nodes.iter().all(|node| !node.name.contains("semantic-")));

    let sc1_node = nodes
        .iter()
        .find(|node| node.name == "Sc1")
        .expect("child Scene member resolves explicit Scene name");
    assert!(sc1_node.has_children);
    let model_node = nodes
        .iter()
        .find(|node| node.name == "Turbine")
        .expect("child Model member resolves explicit Model name");
    assert!(model_node.has_children);

    // Direct Scene activation resolves explicit Scene name instead of Project name.
    let mut scene_app = App::new();
    scene_app.init_resource::<CurrentHierarchyProjection>();
    scene_app.insert_resource(StagePresentationContext {
        root_path: Some("/SceneRoot".to_owned()),
        root_name: Some("Sc3".to_owned()),
        target_names,
    });
    publish_fixture_cache(scene_app.world_mut(), &activation);
    let scene_root = scene_app
        .world()
        .resource::<CurrentHierarchyProjection>()
        .snapshot()
        .nodes
        .iter()
        .find(|node| node.breadcrumb == "/SceneRoot")
        .cloned()
        .expect("direct scene root is published");
    assert_eq!(scene_root.name, "Sc3");

    // Standalone fallback without presentation context falls back to path segment,
    // never to semantic_key.
    let mut standalone_app = App::new();
    standalone_app.init_resource::<CurrentHierarchyProjection>();
    publish_fixture_cache(standalone_app.world_mut(), &activation);
    let standalone_root = standalone_app
        .world()
        .resource::<CurrentHierarchyProjection>()
        .snapshot()
        .nodes
        .iter()
        .find(|node| node.breadcrumb == "/SceneRoot")
        .cloned()
        .expect("standalone root is published");
    assert_eq!(standalone_root.name, "SceneRoot");
}

#[test]
fn incomplete_cache_root_marks_unknown_children_expandable() {
    let scene_id = usd_project::SceneId::new_v4();
    let root = SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::PrimPath("/SceneRoot".into()),
        },
        parent: None,
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: false,
        bim_enabled: false,
        geometry: None,
        material: None,
        animation: None,
        semantic_key: None,
        kind: SceneCacheEntryKind::OwnedPrim {
            prim_path: "/SceneRoot".into(),
        },
        content_hash: None,
    };
    let mut descriptor = crate::cache::SceneCacheDescriptorV3::invalidated(
        scene_id,
        1,
        usd_model::HashDigest::from_hex(&"d".repeat(64)).unwrap(),
    );
    descriptor.state = crate::cache::contract::SceneCacheState::Partial;
    descriptor.prim_count = 12;
    let activation = SceneCacheActivation {
        descriptor,
        index: crate::cache::contract::SceneCacheIndex {
            schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
            scene_id,
            generation: 1,
            entries: vec![root],
        },
    };
    let mut app = App::new();
    app.init_resource::<CurrentHierarchyProjection>();
    publish_fixture_cache(app.world_mut(), &activation);
    let root_node = app
        .world()
        .resource::<CurrentHierarchyProjection>()
        .snapshot()
        .nodes
        .first()
        .cloned()
        .expect("partial cache root is published");
    assert!(root_node.has_children);
}

#[test]
fn resolve_prim_display_name_precedence_covers_explicit_authored_and_fallback() {
    let mut target_names = std::collections::HashMap::new();
    target_names.insert(("scene".to_owned(), "scene-1".to_owned()), "Sc1".to_owned());
    let context = StagePresentationContext {
        root_path: Some("/SceneRoot".to_owned()),
        root_name: Some("Pro3".to_owned()),
        target_names,
    };

    // Root presentation name overrides authored name for the presentation root.
    assert_eq!(
        resolve_prim_display_name(Some(&context), "/SceneRoot", Some("AuthoredRoot"), None)
            .as_deref(),
        Some("Pro3")
    );
    // Explicit target name overrides authored name for Scene/Model members.
    assert_eq!(
        resolve_prim_display_name(
            Some(&context),
            "/SceneRoot/Member_1",
            Some("AuthoredMember"),
            Some(("scene", "scene-1")),
        )
        .as_deref(),
        Some("Sc1")
    );
    // Non-root prims without explicit overrides preserve authored display names.
    assert_eq!(
        resolve_prim_display_name(Some(&context), "/SceneRoot/Chair", Some("Office Chair"), None)
            .as_deref(),
        Some("Office Chair")
    );
    // Whitespace-only authored display names fall back to None (path segment).
    assert_eq!(
        resolve_prim_display_name(Some(&context), "/SceneRoot/Chair", Some("   "), None),
        None
    );
    // Standalone stages without StagePresentationContext preserve authored display name.
    assert_eq!(
        resolve_prim_display_name(None, "/World/Chair", Some("Authored Chair"), None).as_deref(),
        Some("Authored Chair")
    );
}

#[test]
fn out_of_bounds_cache_parent_index_falls_back_to_path_parent_without_panicking() {
    let scene_id = usd_project::SceneId::new_v4();
    let root = SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::PrimPath("/SceneRoot".into()),
        },
        parent: None,
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: false,
        bim_enabled: false,
        geometry: None,
        material: None,
        animation: None,
        semantic_key: None,
        kind: SceneCacheEntryKind::OwnedPrim {
            prim_path: "/SceneRoot".into(),
        },
        content_hash: None,
    };
    let child = SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::PrimPath("/SceneRoot/Part".into()),
        },
        parent: Some(99),
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: false,
        bim_enabled: false,
        geometry: None,
        material: None,
        animation: None,
        semantic_key: None,
        kind: SceneCacheEntryKind::OwnedPrim {
            prim_path: "/SceneRoot/Part".into(),
        },
        content_hash: None,
    };
    let mut descriptor = crate::cache::SceneCacheDescriptorV3::invalidated(
        scene_id,
        1,
        usd_model::HashDigest::from_hex(&"e".repeat(64)).unwrap(),
    );
    descriptor.state = crate::cache::contract::SceneCacheState::Ready;
    descriptor.prim_count = 2;
    let activation = SceneCacheActivation {
        descriptor,
        index: crate::cache::contract::SceneCacheIndex {
            schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
            scene_id,
            generation: 1,
            entries: vec![root, child],
        },
    };
    let mut app = App::new();
    app.init_resource::<CurrentHierarchyProjection>();
    publish_fixture_cache(app.world_mut(), &activation);
    let nodes = app
        .world()
        .resource::<CurrentHierarchyProjection>()
        .snapshot()
        .nodes
        .clone();
    let root_node = nodes
        .iter()
        .find(|node| node.breadcrumb == "/SceneRoot")
        .expect("root node is published");
    assert!(root_node.has_children);
    let child_node = nodes
        .iter()
        .find(|node| node.breadcrumb == "/SceneRoot/Part")
        .expect("child node is published");
    assert_eq!(child_node.parent_id, Some(root_node.id.clone()));
}
