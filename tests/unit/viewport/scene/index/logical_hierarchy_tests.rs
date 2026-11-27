//! Deterministic tests for rendering-independent logical hierarchy authority,
//! cache/source parity, nested composition, non-resident interaction, and
//! eviction preservation.

use std::collections::HashMap;

use bevy::prelude::*;
use tempfile::TempDir;
use usd_project::{SceneId, SceneMember, SceneMemberId, SceneMemberTarget, ScenePlacementTransform};
use viewport_protocol::{
    HierarchyNodeId, HierarchySource, HierarchyVisibilityState, PROTOCOL_VERSION, SceneAnchor,
    ViewportCommand, ViewportCommandEnvelope, ViewportEvent,
};

use super::super::SceneAnchorIndex;
use crate::api::{ViewportCommandInbox, ViewportEventOutbox};
use crate::cache::SceneCacheStore;
use crate::cache::contract::{
    CachedTransform, HierarchyChildCount, SCENE_CACHE_INDEX_SCHEMA_VERSION, SceneCacheAddress,
    SceneCacheDescriptorV3, SceneCacheEntry, SceneCacheEntryKind, SceneCacheIndex,
    SceneCacheOccurrence, SceneCacheState,
};
use crate::cache::scene::hierarchy::{
    LogicalHierarchyCandidate, LogicalOccurrence, LogicalOccurrenceForest, LogicalPrimRecord,
    build_logical_hierarchy_from_cache, build_logical_hierarchy_from_stage,
    validate_logical_hierarchy,
};
use crate::viewport::rendering::scene::{
    CurrentHierarchyProjection, SelectedPrim, SelectedTargets,
};
use crate::viewport::stage::{Spawned, StageInfo, StagePresentationContext};

#[test]
fn canonical_stage_builds_logical_hierarchy_without_ecs_entities_or_meshes() {
    let source = usd_bevy::UsdSnippet::new(
        r#"#usda 1.0
def Xform "SceneRoot" {
    custom string ui:displayName = "Plant"
    def Xform "Wrapper" (
        customData = {
            string "usdhub:hierarchyRole" = "transparentSource"
        }
    ) {
        def Mesh "Pump" {
            custom string ui:displayName = "Main Pump"
            token visibility = "invisible"
        }
    }
    def Xform "Leaf" {}
}
"#,
    )
    .open_stage()
    .expect("open stage fixture");

    let presentation = StagePresentationContext {
        root_path: Some("/SceneRoot".to_owned()),
        root_name: Some("Project Plant".to_owned()),
        target_names: HashMap::new(),
    };
    let records = build_logical_hierarchy_from_stage(&source, Some(&presentation))
        .expect("build logical hierarchy from stage");

    let by_path: HashMap<&str, &LogicalPrimRecord> = records.records
        .iter()
        .map(|record| (record.prim_path.as_str(), record))
        .collect();

    // Transparent wrapper must be collapsed so Pump attaches directly to /SceneRoot.
    assert!(!by_path.contains_key("/SceneRoot/Wrapper"));
    let root = by_path["/SceneRoot"];
    assert_eq!(root.display_name.as_deref(), Some("Project Plant"));
    assert_eq!(root.type_name.as_deref(), Some("Xform"));
    assert_eq!(root.child_count, HierarchyChildCount::Known(2));
    assert!(root.authored_visible);

    let pump = by_path["/SceneRoot/Wrapper/Pump"];
    assert_eq!(
        pump.parent,
        Some(SceneAnchor::active_session("/SceneRoot"))
    );
    assert_eq!(pump.display_name.as_deref(), Some("Main Pump"));
    assert_eq!(pump.type_name.as_deref(), Some("Mesh"));
    assert!(!pump.authored_visible);
    assert_eq!(pump.child_count, HierarchyChildCount::Known(0));

    let leaf = by_path["/SceneRoot/Leaf"];
    assert_eq!(leaf.child_count, HierarchyChildCount::Known(0));
    assert!(!leaf.has_children());

    let root_anchor = root.anchor.clone();
    let pump_anchor = pump.anchor.clone();
    drop(by_path);

    // Install into SceneAnchorIndex with zero ECS entities spawned.
    let mut index = SceneAnchorIndex::default();
    let projection = index.install_logical_hierarchy(records, 1);
    assert!(index.resolve(&pump_anchor).is_none());
    assert!(index.contains_logical_anchor(&pump_anchor));

    // Unified root and child paging works before any ECS entity or mesh exists.
    let roots = projection.prim_roots_read_model().expect("prim roots");
    assert_eq!(roots.total_roots, 1);
    let root_id = HierarchyNodeId::for_scene_anchor(&root_anchor);
    let children = projection
        .children_page(Some(&root_id), 0, 50)
        .expect("child page");
    assert_eq!(children.total, 2);
    assert!(
        children
            .nodes
            .iter()
            .any(|node| node.name == "Main Pump" && !node.visible)
    );
    assert!(index.search_match_for_path("/SceneRoot/Wrapper/Pump").is_some());
}

#[test]
fn cache_and_source_hierarchy_match_for_nested_and_repeated_scene_members() {
    let temp = TempDir::new().expect("create temp project");
    let root = temp.path();
    usd_git::Repository::init(root).expect("init git repo");
    let child_id = SceneId::new_v4();
    let parent_id = SceneId::new_v4();
    let member_a = SceneMemberId::new_v4();
    let member_b = SceneMemberId::new_v4();

    let manifest = usd_project::ProjectManifestV1::new(
        usd_project::ProjectId::new_v4(),
        "Nested Composition Fixture",
        usd_project::ProjectRoot::Scene(parent_id),
        vec![
            usd_project::SceneManifestEntry {
                id: child_id,
                storage_key: usd_project::StorageKey::new("sub-assembly").unwrap(),
                display_name: "Sub Assembly".to_owned(),
            },
            usd_project::SceneManifestEntry {
                id: parent_id,
                storage_key: usd_project::StorageKey::new("parent-assembly").unwrap(),
                display_name: "Parent Assembly".to_owned(),
            },
        ],
        Vec::new(),
    );
    crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(root, &manifest)
        .expect("write project manifest");

    // Author child Scene with authored display name, invisible prim, and transparent wrapper.
    let child_path = crate::project::scene::authoring::scene_path(root, child_id);
    std::fs::create_dir_all(child_path.parent().unwrap()).expect("create scenes dir");
    let child_stage = crate::project::scene::authoring::new_scene_stage(child_id).unwrap();
    let wrapper = child_stage
        .define_prim("/SceneRoot/Wrapper")
        .unwrap()
        .set_type_name("Xform")
        .unwrap();
    wrapper
        .set_metadata(
            "customData",
            openusd::vt::Value::Dictionary(
                [(
                    usd_bevy::USDHUB_HIERARCHY_ROLE_METADATA.to_owned(),
                    openusd::vt::Value::String(usd_bevy::USDHUB_TRANSPARENT_SOURCE_ROLE.to_owned()),
                )]
                .into_iter()
                .collect(),
            ),
        )
        .unwrap();
    let valve = child_stage
        .define_prim("/SceneRoot/Wrapper/Valve")
        .unwrap()
        .set_type_name("Xform")
        .unwrap();
    valve
        .clone()
        .set_metadata(
            "ui:displayName",
            openusd::vt::Value::String("Relief Valve".to_owned()),
        )
        .unwrap();
    usd_bevy::authoring::set_attribute(
        &child_stage,
        "/SceneRoot/Wrapper/Valve",
        "visibility",
        "token",
        openusd::vt::Value::Token(openusd::tf::Token::new("invisible")),
    )
    .unwrap();
    let exported = usd_bevy::authoring::export_stage_string(&child_stage)
        .expect("export child stage string");
    std::fs::write(&child_path, exported).expect("write child scene");

    // Author parent Scene referencing the same child Scene twice under distinct member IDs.
    let members = vec![
        SceneMember {
            id: member_a,
            target: SceneMemberTarget::Scene(child_id),
            name: Some("Sub A".to_owned()),
            transform: ScenePlacementTransform::IDENTITY,
        },
        SceneMember {
            id: member_b,
            target: SceneMemberTarget::Scene(child_id),
            name: Some("Sub B".to_owned()),
            transform: ScenePlacementTransform::IDENTITY,
        },
    ];
    let parent_path =
        crate::project::scene::authoring::author_scene_atomic_with_members(root, parent_id, &members)
            .expect("author parent scene");

    let member_a_path = crate::project::scene::authoring::scene_member_path(member_a);
    let member_b_path = crate::project::scene::authoring::scene_member_path(member_b);
    let valve_a_path = format!("{member_a_path}/Wrapper/Valve");
    let valve_b_path = format!("{member_b_path}/Wrapper/Valve");
    // An override authored at the parent occurrence must not be copied from
    // the standalone child cache or from the other repeated occurrence.
    let parent_edit = usd_bevy::LiveStage::open_document(&parent_path).unwrap();
    usd_bevy::authoring::set_attribute(
        &parent_edit.stage, &valve_a_path, "visibility", "token",
        openusd::vt::Value::Token(openusd::tf::Token::new("inherited")),
    ).unwrap();
    usd_bevy::authoring::save_stage_as(&parent_edit.stage, parent_path.to_str().unwrap()).unwrap();
    drop(parent_edit);

    // Build and publish verified Scene caches for both child and parent.
    let config_hash = crate::cache::hydration::default_project_cache_config_hash();
    let store = SceneCacheStore::new(root);
    let publish_scene = |scene_id: SceneId, scene_file: &std::path::Path| {
        let generation = store.advance_generation(scene_id, config_hash).unwrap();
        let live = usd_bevy::LiveStage::open_document(scene_file).unwrap();
        let identity = crate::cache::identity::scene_revision_identity_from_snapshot(
            root,
            scene_id,
            viewport_protocol::RuntimeProfile::NativeMedium,
            config_hash,
            &live,
        )
        .unwrap();
        let (index, spatial) = crate::cache::scene::build::build_scene_cache_index_from_snapshot(
            root, scene_id, generation, &live,
        )
        .unwrap();
        let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, generation, config_hash);
        descriptor.state = SceneCacheState::Ready;
        descriptor.prim_count = index.entries.len() as u64;
        descriptor.prim_count_ready = true;
        descriptor.source_content_hash = Some(identity.source_closure_digest);
        descriptor.revision_identity = Some(identity.digest());
        store
            .publish_generation(&descriptor, &index, &spatial)
            .unwrap();
    };
    publish_scene(child_id, &child_path);
    publish_scene(parent_id, &parent_path);

    let presentation = StagePresentationContext {
        root_path: Some("/SceneRoot".to_owned()),
        root_name: Some("Parent Assembly".to_owned()),
        target_names: HashMap::from([(
            ("scene".to_owned(), child_id.to_string()),
            "Sub Assembly".to_owned(),
        )]),
    };

    let parent_activation = store
        .load_activation(parent_id)
        .unwrap()
        .expect("verified parent activation");
    let cached_records =
        build_logical_hierarchy_from_cache(&parent_activation, Some(root), Some(&presentation))
            .expect("build composed logical hierarchy from cache");

    let parent_live = usd_bevy::LiveStage::open_document(&parent_path).unwrap();
    let source_records = build_logical_hierarchy_from_stage(&parent_live.stage, Some(&presentation))
        .expect("build composed logical hierarchy from source");

    let cached_map: HashMap<&str, &LogicalPrimRecord> = cached_records.records
        .iter()
        .map(|record| (record.prim_path.as_str(), record))
        .collect();
    let source_map: HashMap<&str, &LogicalPrimRecord> = source_records.records
        .iter()
        .map(|record| (record.prim_path.as_str(), record))
        .collect();

    // Both repeated occurrences exist with distinct occurrence paths, collapsed wrappers,
    // identical display names, and identical authored visibility across cache and source.
    for path in [&member_a_path, &member_b_path, &valve_a_path, &valve_b_path] {
        let cached = cached_map
            .get(path.as_str())
            .unwrap_or_else(|| panic!("missing cached node {path}"));
        let source = source_map
            .get(path.as_str())
            .unwrap_or_else(|| panic!("missing source node {path}"));
        assert_eq!(cached.display_name, source.display_name, "display_name for {path}");
        assert_eq!(
            cached.authored_visible, source.authored_visible,
            "authored_visible for {path}"
        );
        assert_eq!(
            cached.has_children(),
            source.has_children(),
            "has_children for {path}"
        );
    }
    assert_eq!(
        cached_map[valve_a_path.as_str()].parent,
        Some(SceneAnchor::active_session(member_a_path.clone()))
    );
    assert_eq!(
        cached_map[valve_b_path.as_str()].parent,
        Some(SceneAnchor::active_session(member_b_path.clone()))
    );
    assert_eq!(
        cached_map[valve_a_path.as_str()].display_name.as_deref(),
        Some("Relief Valve")
    );
    assert!(cached_map[valve_a_path.as_str()].authored_visible);
    assert!(!cached_map[valve_b_path.as_str()].authored_visible);

    // Member rows are captured from this parent revision, so no child cache
    // lookup is necessary. A missing branch snapshot still remains Unknown.
    let without_child_store = build_logical_hierarchy_from_cache(
        &parent_activation, None, Some(&presentation),
    ).unwrap();
    assert_eq!(cached_records, without_child_store);
    let mut incomplete = parent_activation.clone();
    let member = incomplete.index.entries.iter_mut().find(|entry| {
        matches!(&entry.kind, SceneCacheEntryKind::ChildScene { member_id, .. } if *member_id == member_a)
    }).unwrap();
    member.hierarchy.composed_descendants = None;
    let incomplete_rows = build_logical_hierarchy_from_cache(&incomplete, None, Some(&presentation)).unwrap();
    let member_row = incomplete_rows.records.iter().find(|record| record.prim_path == member_a_path).unwrap();
    assert_eq!(member_row.child_count, HierarchyChildCount::Unknown);
    assert!(!incomplete_rows.records.iter().any(|row| row.prim_path == valve_a_path));
    assert!(incomplete_rows.records.iter().any(|row| row.prim_path == valve_b_path));

    let mut unavailable = parent_activation.clone();
    let member = unavailable.index.entries.iter_mut().find(|entry| {
        matches!(&entry.kind, SceneCacheEntryKind::ChildScene { member_id, .. } if *member_id == member_a)
    }).unwrap();
    member.hierarchy.metadata_populated = false;
    assert!(build_logical_hierarchy_from_cache(&unavailable, None, Some(&presentation)).is_err());

    let mut unresolved_wrapper = parent_activation.clone();
    let member = unresolved_wrapper.index.entries.iter_mut().find(|entry| {
        matches!(&entry.kind, SceneCacheEntryKind::ChildScene { member_id, .. } if *member_id == member_a)
    }).unwrap();
    let wrapper_path = format!("{member_a_path}/Wrapper");
    let wrapper = member.hierarchy.composed_descendants.as_mut().unwrap().iter_mut()
        .find(|row| row.prim_path == wrapper_path).unwrap();
    wrapper.metadata.child_count = HierarchyChildCount::Unknown;
    let rows = build_logical_hierarchy_from_cache(&unresolved_wrapper, None, Some(&presentation)).unwrap();
    let wrapper = rows.records.iter().find(|row| row.prim_path == wrapper_path).unwrap();
    assert_eq!(wrapper.child_count, HierarchyChildCount::Unknown);
    assert!(wrapper.has_children());

    // The displayed unresolved wrapper is a valid scope root. Resolving it
    // collapses only its row, not its occurrence or command.
    let wrapper_anchor = SceneAnchor::active_session(wrapper_path.clone());
    let valve_a = SceneAnchor::active_session(valve_a_path.clone());
    let valve_b = SceneAnchor::active_session(valve_b_path.clone());
    let mut index = SceneAnchorIndex::default();
    index.install_logical_hierarchy(rows, 1);
    assert!(index.set_logical_subtree_visibility(&wrapper_anchor, false));
    index.install_logical_hierarchy(source_records.clone(), 1);
    assert!(!index.contains_logical_anchor(&wrapper_anchor));
    assert_eq!(index.logical_visibility(&valve_a), Some(false));
    assert_eq!(index.visibility_override(&valve_b), None);
    assert_eq!(index.visibility_override(&wrapper_anchor), Some(false));
    let mut world = World::new();
    let support_entity = world.spawn_empty().id();
    let valve_entity = world.spawn_empty().id();
    let (intents, unmapped) = index.bind_resident_entities_to_logical([
        (support_entity, wrapper_path.clone(), true),
        (valve_entity, valve_a_path.clone(), false),
    ]);
    assert!(unmapped.is_empty());
    assert!(intents.contains(&(support_entity, false)));
    assert!(intents.contains(&(valve_entity, false)));
    index.evict_resident_entity(valve_entity);
    let re_admitted = world.spawn_empty().id();
    let (intents, unmapped) = index.bind_resident_entities_to_logical([
        (re_admitted, valve_a_path.clone(), false),
    ]);
    assert!(unmapped.is_empty());
    assert_eq!(intents, vec![(re_admitted, false)]);
    assert!(index.set_logical_subtree_visibility(&valve_a, true));
    assert_eq!(index.logical_visibility(&valve_a), Some(true));
    index.install_logical_hierarchy(source_records, 2);
    assert_eq!(index.logical_visibility(&valve_a), Some(true));
    assert_eq!(index.visibility_override(&wrapper_anchor), None);
}

#[test]
fn cache_index_and_logical_hierarchy_validators_reject_cycles_and_duplicates() {
    let scene_id = SceneId::new_v4();
    let entry_a = SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::PrimPath("/SceneRoot/A".into()),
        },
        parent: Some(1),
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: false,
        bim_enabled: false,
        geometry: None,
        material: None,
        animation: None,
        semantic_key: None,
        kind: SceneCacheEntryKind::OwnedPrim {
            prim_path: "/SceneRoot/A".into(),
        },
        content_hash: None,
        hierarchy: Default::default(),
    };
    let entry_b = SceneCacheEntry {
        address: SceneCacheAddress {
            scene_id,
            occurrence: SceneCacheOccurrence::PrimPath("/SceneRoot/B".into()),
        },
        parent: Some(0),
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: false,
        bim_enabled: false,
        geometry: None,
        material: None,
        animation: None,
        semantic_key: None,
        kind: SceneCacheEntryKind::OwnedPrim {
            prim_path: "/SceneRoot/B".into(),
        },
        content_hash: None,
        hierarchy: Default::default(),
    };
    let cyclic = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 1,
        entries: vec![entry_a, entry_b],
    };
    assert!(cyclic.validate(scene_id, 1).is_err());

    let duplicate = LogicalPrimRecord {
        anchor: SceneAnchor::active_session("/World/A"),
        parent: None,
        prim_path: "/World/A".to_owned(),
        label: "A".to_owned(),
        display_name: None,
        type_name: Some("Xform".to_owned()),
        child_count: HierarchyChildCount::Known(0),
        authored_visible: true,
        geometry_ref: None,
    };
    assert!(validate_logical_hierarchy(&[duplicate.clone(), duplicate]).is_err());
}

fn fixture_candidate(records: Vec<LogicalPrimRecord>, canonical: bool) -> LogicalHierarchyCandidate {
    let occurrences = records.iter().map(|record| {
        (record.anchor.clone(), LogicalOccurrence {
            parent: record.parent.clone(),
            children_unknown: record.child_count == HierarchyChildCount::Unknown,
            authored_visible: record.authored_visible,
        })
    }).collect();
    LogicalHierarchyCandidate {
        records,
        forest: LogicalOccurrenceForest { occurrences, canonical },
    }
}

fn logical_hierarchy_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, crate::api::ViewportBridgePlugin));
    app.init_resource::<usd_bevy::PendingStageChanges>();
    app.init_resource::<Spawned>();
    app.init_resource::<StageInfo>();
    app.init_resource::<SelectedPrim>();
    app.init_resource::<crate::viewport::navigation::camera::CameraMount>();
    app.init_resource::<crate::viewport::navigation::camera::CameraOrientationState>();
    app.init_resource::<crate::viewport::navigation::camera::FlyTo>();
    app.init_resource::<crate::animation::UsdStageTime>();
    app.init_resource::<crate::viewport::rendering::scene::visualization::DisplayToggles>();
    app.init_resource::<crate::viewport::stage::LoaderTuning>();
    app.init_resource::<crate::physics::PhysicsActive>();
    app.init_resource::<crate::viewport::stage::ReloadRequest>();
    app.init_resource::<crate::viewport::rendering::sampling::DlssCapability>();
    app.init_resource::<crate::viewport::rendering::sampling::SamplingCoordinatorState>();
    app.init_resource::<crate::viewport::rendering::sampling::DlssCameraActivation>();
    app.world_mut().resource_mut::<StageInfo>().activation_generation = 1;
    app
}

#[test]
fn logical_selection_visibility_intents_apply_on_admission_and_survive_eviction() {
    let mut app = logical_hierarchy_app();
    let root_anchor = SceneAnchor::active_session("/World");
    let child_anchor = SceneAnchor::active_session("/World/Part");
    let records = vec![
        LogicalPrimRecord {
            anchor: root_anchor.clone(),
            parent: None,
            prim_path: "/World".to_owned(),
            label: "World".to_owned(),
            display_name: None,
            type_name: Some("Xform".to_owned()),
            child_count: HierarchyChildCount::Known(1),
            authored_visible: true,
            geometry_ref: None,
        },
        LogicalPrimRecord {
            anchor: child_anchor.clone(),
            parent: Some(root_anchor.clone()),
            prim_path: "/World/Part".to_owned(),
            label: "Part".to_owned(),
            display_name: Some("Engine Block".to_owned()),
            type_name: Some("Mesh".to_owned()),
            child_count: HierarchyChildCount::Known(0),
            authored_visible: true,
            geometry_ref: None,
        },
    ];

    let projection = app
        .world_mut()
        .resource_mut::<SceneAnchorIndex>()
        .install_logical_hierarchy(fixture_candidate(records, true), 1);
    *app.world_mut().resource_mut::<CurrentHierarchyProjection>() = projection;

    // 1. Before any ECS entity exists, select and hide /World/Part logically.
    {
        let mut inbox = app.world_mut().resource_mut::<ViewportCommandInbox>();
        inbox.push(ViewportCommandEnvelope::new(
            "req-select",
            ViewportCommand::SelectTarget {
                target: Some(child_anchor.clone()),
            },
        ));
        inbox.push(ViewportCommandEnvelope::new(
            "req-hide",
            ViewportCommand::SetSubtreeVisibility {
                target: child_anchor.clone(),
                visible: false,
            },
        ));
    }
    app.update();

    assert_eq!(
        app.world().resource::<SelectedTargets>().0.primary.as_ref(),
        Some(&child_anchor)
    );
    assert!(app.world().resource::<SelectedPrim>().0.is_none());
    assert_eq!(
        app.world()
            .resource::<SceneAnchorIndex>()
            .visibility_override(&child_anchor),
        Some(false)
    );
    assert_eq!(
        app.world()
            .resource::<SceneAnchorIndex>()
            .visibility_for_anchor(&child_anchor),
        HierarchyVisibilityState::Hidden
    );
    let events: Vec<_> = {
        let mut outbox = app.world_mut().resource_mut::<ViewportEventOutbox>();
        std::iter::from_fn(|| outbox.pop()).collect()
    };
    assert!(events.iter().any(|env| matches!(
        &env.event,
        ViewportEvent::PrimVisibilityChanged { target, visible }
            if target == &child_anchor && !*visible
    )));

    // 2. Admit the ECS entity later -> recorded visibility intent and SelectedPrim apply automatically.
    let entity = app
        .world_mut()
        .spawn((usd_bevy::UsdPrimRef::new("/World/Part"), Visibility::Visible))
        .id();
    app.update();

    assert_eq!(
        app.world().resource::<SceneAnchorIndex>().resolve(&child_anchor),
        Some(entity)
    );
    assert_eq!(app.world().resource::<SelectedPrim>().0, Some(entity));
    assert_eq!(
        app.world().get::<Visibility>(entity),
        Some(&Visibility::Hidden)
    );

    // 3. Evict (despawn) the ECS entity -> resident mapping clears, logical row & intents survive!
    app.world_mut().despawn(entity);
    app.update();

    assert!(
        app.world()
            .resource::<SceneAnchorIndex>()
            .resolve(&child_anchor)
            .is_none()
    );
    assert!(
        app.world()
            .resource::<SceneAnchorIndex>()
            .contains_logical_anchor(&child_anchor)
    );
    assert_eq!(
        app.world().resource::<SelectedTargets>().0.primary.as_ref(),
        Some(&child_anchor)
    );
    assert_eq!(
        app.world()
            .resource::<SceneAnchorIndex>()
            .visibility_override(&child_anchor),
        Some(false)
    );
    let root_id = HierarchyNodeId::for_scene_anchor(&root_anchor);
    let page = app
        .world()
        .resource::<CurrentHierarchyProjection>()
        .children_page(Some(&root_id), 0, 50)
        .expect("logical row survives eviction");
    assert_eq!(page.total, 1);
    assert_eq!(page.nodes[0].name, "Engine Block");
    assert!(!page.nodes[0].visible);

    // 4. Re-admit a new ECS entity for the same occurrence -> hidden intent re-applies.
    let re_admitted = app
        .world_mut()
        .spawn((usd_bevy::UsdPrimRef::new("/World/Part"), Visibility::Visible))
        .id();
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(re_admitted),
        Some(&Visibility::Hidden)
    );
}


#[test]
fn canonical_replacement_and_mutation_do_not_depend_on_residency() {
    let mut app = logical_hierarchy_app();
    let root = SceneAnchor::active_session("/World");
    let removed = SceneAnchor::active_session("/World/Removed");
    let part = SceneAnchor::active_session("/World/Part");
    let provisional = vec![LogicalPrimRecord {
        anchor: root.clone(), parent: None, prim_path: root.prim_path.clone(),
        label: "World".to_owned(), display_name: None, type_name: Some("Xform".to_owned()),
        child_count: HierarchyChildCount::Unknown, authored_visible: true, geometry_ref: None,
    }];
    let projection = app.world_mut().resource_mut::<SceneAnchorIndex>()
        .install_logical_hierarchy(fixture_candidate(provisional, false), 1);
    *app.world_mut().resource_mut::<CurrentHierarchyProjection>() = projection;
    assert!(app.world_mut().resource_mut::<SceneAnchorIndex>()
        .set_logical_subtree_visibility(&root, false));

    let stage = usd_bevy::UsdSnippet::new(r#"#usda 1.0
 def Xform "World" {
     def Xform "Part" {}
     def Xform "Removed" {}
 }
 "#).open_stage().unwrap();
    app.world_mut().insert_non_send(usd_bevy::LiveStage::new(stage));
    app.update();
    {
        let index = app.world().resource::<SceneAnchorIndex>();
        assert!(index.contains_logical_anchor(&part));
        assert!(index.resolve(&part).is_none());
        assert_eq!(index.visibility_override(&part), Some(false));
        assert_eq!(index.logical_visibility(&part), Some(false));
    }
    app.world_mut().resource_mut::<SelectedTargets>().replace(
        viewport_protocol::SelectionReadModel { targets: vec![removed.clone()], primary: Some(removed.clone()) }
    ).unwrap();
    assert!(app.world_mut().resource_mut::<SceneAnchorIndex>()
        .set_logical_subtree_visibility(&removed, true));
    {
        let live = app.world().get_non_send::<usd_bevy::LiveStage>().unwrap();
        assert!(usd_bevy::authoring::remove_prim(&live.stage, &removed.prim_path).unwrap());
        usd_bevy::authoring::define_prim(&live.stage, "/World/Part/New", "Xform").unwrap();
    }
    app.update();
    let added = SceneAnchor::active_session("/World/Part/New");
    {
        let index = app.world().resource::<SceneAnchorIndex>();
        assert!(!index.contains_logical_anchor(&removed));
        assert!(index.search_match_for_path(&removed.prim_path).is_none());
        assert!(index.contains_logical_anchor(&added));
        assert_eq!(index.visibility_override(&added), Some(false));
        assert!(index.resolve(&added).is_none());
    }
    assert!(app.world().resource::<SelectedTargets>().0.targets.is_empty());
    // Recreating the deleted textual path must not resurrect its newer show
    // scope; only the surviving ancestor's hide applies.
    {
        let live = app.world().get_non_send::<usd_bevy::LiveStage>().unwrap();
        usd_bevy::authoring::define_prim(&live.stage, &removed.prim_path, "Xform").unwrap();
    }
    app.update();
    assert_eq!(app.world().resource::<SceneAnchorIndex>().logical_visibility(&removed), Some(false));
    let entity = app.world_mut().spawn((usd_bevy::UsdPrimRef::new("/World/Part/New"), Visibility::Visible)).id();
    app.update();
    assert_eq!(app.world().get::<Visibility>(entity), Some(&Visibility::Hidden));
    app.world_mut().despawn(entity);
    app.update();
    assert!(app.world().resource::<SceneAnchorIndex>().contains_logical_anchor(&added));

    // Parent/child ordering survives subsequent canonical replacement.
    let mut index = app.world_mut().resource_mut::<SceneAnchorIndex>();
    assert!(index.set_logical_subtree_visibility(&part, true));
    assert_eq!(index.visibility_override(&added), Some(true));
    assert!(index.set_logical_subtree_visibility(&root, false));
    assert_eq!(index.visibility_override(&added), Some(false));
    let _ = index.install_logical_hierarchy(fixture_candidate(Vec::new(), true), 2);
    assert!(index.has_logical_authority());
    assert!(!index.contains_logical_anchor(&added));
    assert_eq!(index.visibility_override(&added), None);
}

#[test]
fn unresolved_and_failed_candidates_defer_intent_but_confirmed_deletion_clears_it() {
    let mut index = SceneAnchorIndex::default();
    let root = SceneAnchor::active_session("/World");
    let child = SceneAnchor::active_session("/World/Part");
    let root_record = LogicalPrimRecord {
        anchor: root.clone(), parent: None, prim_path: root.prim_path.clone(),
        label: "World".into(), display_name: None, type_name: Some("Xform".into()),
        child_count: HierarchyChildCount::Known(1), authored_visible: true, geometry_ref: None,
    };
    let child_record = LogicalPrimRecord {
        anchor: child.clone(), parent: Some(root.clone()), prim_path: child.prim_path.clone(),
        label: "Part".into(), display_name: None, type_name: Some("Xform".into()),
        child_count: HierarchyChildCount::Known(0), authored_visible: true, geometry_ref: None,
    };
    let complete = fixture_candidate(vec![root_record.clone(), child_record.clone()], true);
    index.install_logical_hierarchy(complete.clone(), 1);
    assert!(index.set_logical_subtree_visibility(&child, false));
    let mut world = World::new();
    let resident = world.spawn_empty().id();
    let (intents, unmapped) = index.bind_resident_entities_to_logical([
        (resident, child.prim_path.clone(), false),
    ]);
    assert!(unmapped.is_empty());
    assert_eq!(intents, vec![(resident, false)]);
    assert_eq!(index.resolve(&child), Some(resident));
    let mut unresolved_root = root_record.clone();
    unresolved_root.child_count = HierarchyChildCount::Unknown;
    index.install_logical_hierarchy(fixture_candidate(vec![unresolved_root], true), 1);
    assert!(!index.contains_logical_anchor(&child));
    assert!(index.retains_logical_selection(&child));
    assert_eq!(index.logical_visibility(&child), None);
    assert!(index.resolve(&child).is_none());
    assert!(index.anchor_for(resident).is_none());
    assert!(!index.set_logical_subtree_visibility(&child, true));
    index.install_logical_hierarchy(complete.clone(), 1);
    assert_eq!(index.logical_visibility(&child), Some(false));
    let (intents, unmapped) = index.bind_resident_entities_to_logical([
        (resident, child.prim_path.clone(), false),
    ]);
    assert!(unmapped.is_empty());
    assert_eq!(intents, vec![(resident, false)]);
    index.mark_logical_hierarchy_unavailable(1);
    assert!(index.has_logical_authority());
    assert!(!index.logical_hierarchy_available());
    assert!(!index.contains_logical_anchor(&child));
    assert!(index.retains_logical_selection(&child));
    assert!(index.resolve(&child).is_none());
    assert!(index.anchor_for(resident).is_none());
    assert!(!index.accepts_logical_prim_path(&child.prim_path));
    assert!(!index.set_logical_subtree_visibility(&child, true));
    index.install_logical_hierarchy(complete.clone(), 1);
    assert_eq!(index.logical_visibility(&child), Some(false));

    let mut empty_root = root_record;
    empty_root.child_count = HierarchyChildCount::Known(0);
    index.install_logical_hierarchy(fixture_candidate(vec![empty_root], true), 1);
    assert!(!index.retains_logical_selection(&child));
    index.install_logical_hierarchy(complete.clone(), 1);
    assert_eq!(index.logical_visibility(&child), Some(true));
    assert_eq!(index.visibility_override(&child), None);
    assert!(index.set_logical_subtree_visibility(&child, false));
    index.install_logical_hierarchy(complete, 2);
    assert_eq!(index.logical_visibility(&child), Some(true));
}
