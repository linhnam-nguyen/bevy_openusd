use super::*;

fn node(
    path: &str,
    parent: Option<SceneAnchor>,
    visible: bool,
    has_children: bool,
) -> PrimNodeReadModel {
    PrimNodeReadModel {
        anchor: SceneAnchor::active_session(path),
        parent,
        label: path.rsplit('/').next().unwrap_or(path).to_owned(),
        display_name: Some(path.rsplit('/').next().unwrap_or(path).to_owned()),
        visible,
        has_children,
    }
}

#[test]
fn snapshot_and_incremental_events_produce_the_authoritative_projection() {
    let mut state = ViewportReadModelState::default();
    let mut snapshot = ViewportReadModel::unloaded("fixture.usda");
    snapshot.presentation.ground_grid = true;
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(snapshot.clone()),
        },
    ));

    state.apply(&ViewportEventEnvelope::new(
        Some("local-1".into()),
        ViewportEvent::PhysicsChanged { running: true },
    ));
    state.apply(&ViewportEventEnvelope::new(
        Some("local-2".into()),
        ViewportEvent::PresentationChanged {
            presentation: viewport_protocol::PresentationReadModel {
                wireframe: true,
                ..snapshot.presentation.clone()
            },
        },
    ));
    let mut viewer_settings = viewport_protocol::ViewerSettingsReadModel::default();
    viewer_settings.environment.grid_color = viewport_protocol::ColorRgb8::new(1, 2, 3);
    state.apply(&ViewportEventEnvelope::new(
        Some("local-3".into()),
        ViewportEvent::ViewerSettingsChanged {
            settings: viewer_settings.clone(),
        },
    ));

    let current = state.snapshot().expect("snapshot is available");
    assert!(current.physics_running);
    assert!(current.presentation.ground_grid);
    assert!(current.presentation.wireframe);
    assert_eq!(current.viewer_settings, viewer_settings);
}

#[test]
fn paged_tree_search_and_visibility_reduce_without_ecs_state() {
    let mut state = ViewportReadModelState::default();
    let root = node("/World", None, true, true);
    let door = node("/World/Door", Some(root.anchor.clone()), true, false);
    let handle = node("/World/Door/Handle", Some(door.anchor.clone()), true, false);
    let mut snapshot = ViewportReadModel::unloaded("fixture.usdz");
    snapshot.stage.loaded = true;
    snapshot.scene.prims = vec![root.clone()];
    snapshot.scene.total_prims = 3;
    snapshot.scene.total_roots = 1;
    snapshot.scene.root_page_size = DEFAULT_SCENE_PAGE_SIZE;

    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(snapshot),
        },
    ));
    assert_eq!(state.scene_nodes(), vec![root.clone()]);

    state.request_scene_children(root.anchor.clone());
    assert_eq!(
        state.take_scene_page_requests(),
        vec![ScenePageRequest {
            parent: Some(root.anchor.clone()),
            page: 0,
            page_size: DEFAULT_SCENE_PAGE_SIZE,
        }]
    );
    state.apply(&ViewportEventEnvelope::new(
        Some("local-2".into()),
        ViewportEvent::SceneChildren {
            page: SceneChildrenPage {
                parent: Some(root.anchor.clone()),
                page: 0,
                page_size: DEFAULT_SCENE_PAGE_SIZE,
                total: 1,
                nodes: vec![door.clone(), handle.clone()],
            },
        },
    ));
    assert_eq!(state.scene_nodes().len(), 3);

    state.apply(&ViewportEventEnvelope::new(
        Some("local-3".into()),
        ViewportEvent::PrimVisibilityChanged {
            target: root.anchor.clone(),
            visible: false,
        },
    ));
    assert!(state.scene_nodes().iter().all(|node| !node.visible));

    state.begin_search("local-4".into(), "door".into());
    state.apply(&ViewportEventEnvelope::new(
        Some("stale".into()),
        ViewportEvent::SearchResults {
            query: "door".into(),
            offset: 0,
            total: 1,
            matches: vec![],
            has_more: false,
        },
    ));
    assert!(state.search_results().is_empty());
    state.apply(&ViewportEventEnvelope::new(
        Some("local-4".into()),
        ViewportEvent::SearchResults {
            query: "door".into(),
            offset: 0,
            total: 1,
            matches: vec![viewport_protocol::SceneSearchMatch {
                anchor: door.anchor.clone(),
                parent: Some(root.anchor.clone()),
                label: door.label.clone(),
                breadcrumb: door.anchor.prim_path.clone(),
                visible: false,
                has_children: false,
                reveal_pages: vec![],
            }],
            has_more: false,
        },
    ));
    assert_eq!(state.search_results().len(), 1);
    assert_eq!(state.search_status(), Some((1, false)));
    assert_eq!(state.next_search_page(), None);
}

fn loaded_snapshot(
    stage_name: &str,
    activation_generation: u64,
    selection_targets: Vec<SceneAnchor>,
    selection_revision: u64,
) -> ViewportReadModel {
    let mut snapshot = ViewportReadModel::unloaded(stage_name);
    snapshot.stage.loaded = true;
    snapshot.stage.activation_generation = activation_generation;
    snapshot.selection.primary = selection_targets.first().cloned();
    snapshot.selection.targets = selection_targets;
    snapshot.selection_revision = selection_revision;
    snapshot
}

fn make_bim_property(key: &str, value: &str) -> viewport_protocol::BimPropertyReadModel {
    viewport_protocol::BimPropertyReadModel {
        key: key.to_owned(),
        label: key.to_owned(),
        scope: usd_model::BimPropertyScope::Instance,
        group_id: BimPropertyGroupId::Instance,
        value: viewport_protocol::CommonValue::Same(usd_model::CanonicalValue::Text(
            value.to_owned(),
        )),
        target_values: Vec::new(),
        measurement: None,
        units: Vec::new(),
        current_display_unit: None,
        editable: false,
    }
}

fn make_bim_properties_model(
    targets: Vec<SceneAnchor>,
    selection_revision: u64,
    group_id: BimPropertyGroupId,
    group_name: &str,
    props: Vec<(&str, &str)>,
) -> BimPropertiesReadModel {
    BimPropertiesReadModel {
        targets,
        selection_revision,
        groups: vec![BimPropertyGroupReadModel {
            id: group_id,
            name: group_name.to_owned(),
            editable_group: false,
            properties: props
                .into_iter()
                .map(|(k, v)| make_bim_property(k, v))
                .collect(),
        }],
    }
}

fn make_catalogue(revision: u64, count: usize) -> BimClassificationFieldCatalogue {
    BimClassificationFieldCatalogue {
        semantic_revision: revision,
        fields: (0..count)
            .map(|i| {
                viewport_protocol::BimClassificationFieldDescriptor::new(
                    viewport_protocol::BimFieldKey::property(format!("field_{i}")),
                    format!("Field {i}"),
                    usd_model::BimPropertyScope::Instance,
                )
            })
            .collect(),
    }
}

#[test]
fn bim_properties_ready_selection_accepts_matching_request_and_clears_on_selection_change() {
    let mut state = ViewportReadModelState::default();
    let a = SceneAnchor::active_session("/World/A");
    let b = SceneAnchor::active_session("/World/B");

    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(loaded_snapshot("s.usda", 1, vec![a.clone()], 1)),
        },
    ));
    assert_eq!(state.bim_properties_state(), BimPropertiesLoadState::NotRequested);

    state
        .begin_bim_properties_request("local-10".to_owned())
        .expect("loaded stage with selection accepts request");
    assert_eq!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Loading {
            received_pages: 0,
            total_pages: 0
        }
    );

    let props_a = make_bim_properties_model(
        vec![a.clone()],
        1,
        BimPropertyGroupId::Instance,
        "Instance",
        vec![("Mark", "A1")],
    );
    state.apply(&ViewportEventEnvelope::new(
        Some("local-10".to_owned()),
        ViewportEvent::BimPropertiesRead {
            properties: props_a.clone(),
            diff: None,
        },
    ));
    assert_eq!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Ready(&props_a)
    );

    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::SelectionChanged {
            selection: viewport_protocol::SelectionReadModel {
                primary: Some(b.clone()),
                targets: vec![b],
                ..Default::default()
            },
        },
    ));
    assert!(state.bim_properties().is_none());
    assert_eq!(state.bim_properties_state(), BimPropertiesLoadState::NotRequested);
}

#[test]
fn bim_properties_reordered_responses_and_stale_errors_fail_closed() {
    let mut state = ViewportReadModelState::default();
    let a = SceneAnchor::active_session("/World/A");
    let b = SceneAnchor::active_session("/World/B");
    let c = SceneAnchor::active_session("/World/C");

    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(loaded_snapshot("s.usda", 1, vec![a.clone()], 1)),
        },
    ));
    state
        .begin_bim_properties_request("local-10".to_owned())
        .unwrap();

    // Switch selection to B and issue request 11 before request 10 arrives.
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(loaded_snapshot("s.usda", 1, vec![b.clone()], 2)),
        },
    ));
    state
        .begin_bim_properties_request("local-11".to_owned())
        .unwrap();

    // Late A response arrives for B -> must be ignored.
    let props_a = make_bim_properties_model(
        vec![a],
        1,
        BimPropertyGroupId::Instance,
        "Instance",
        vec![("Mark", "A1")],
    );
    state.apply(&ViewportEventEnvelope::new(
        Some("local-10".to_owned()),
        ViewportEvent::BimPropertiesRead {
            properties: props_a,
            diff: None,
        },
    ));
    assert!(state.bim_properties().is_none());
    assert_eq!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Loading {
            received_pages: 0,
            total_pages: 0
        }
    );

    // Matching B response arrives -> accepted.
    let props_b = make_bim_properties_model(
        vec![b.clone()],
        2,
        BimPropertyGroupId::Instance,
        "Instance",
        vec![("Mark", "B2")],
    );
    state.apply(&ViewportEventEnvelope::new(
        Some("local-11".to_owned()),
        ViewportEvent::BimPropertiesRead {
            properties: props_b.clone(),
            diff: None,
        },
    ));
    assert_eq!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Ready(&props_b)
    );

    // Stale A error arrives after B succeeded -> must NOT clear B or show error.
    state.apply(&ViewportEventEnvelope::new(
        Some("local-10".to_owned()),
        ViewportEvent::BimPropertiesError {
            error: viewport_protocol::BimPropertiesDeliveryError {
                selection_revision: 1,
                kind: viewport_protocol::BimPropertiesDeliveryErrorKind::TooManyProperties,
                property: None,
                encoded_bytes: 999,
                max_bytes: 512,
            },
        },
    ));
    assert_eq!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Ready(&props_b)
    );

    // Switch to C, request 12, fail current request -> no B fallback, error visible.
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(loaded_snapshot("s.usda", 1, vec![c], 3)),
        },
    ));
    state
        .begin_bim_properties_request("local-12".to_owned())
        .unwrap();
    state.apply(&ViewportEventEnvelope::new(
        Some("local-12".to_owned()),
        ViewportEvent::BimPropertiesError {
            error: viewport_protocol::BimPropertiesDeliveryError {
                selection_revision: 3,
                kind: viewport_protocol::BimPropertiesDeliveryErrorKind::OversizedMetadata,
                property: None,
                encoded_bytes: 2048,
                max_bytes: 1024,
            },
        },
    ));
    assert!(state.bim_properties().is_none());
    assert!(matches!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Error(_)
    ));
}

#[test]
fn bim_stage_reload_same_path_and_stage_switch_invalidate_properties_and_catalogue() {
    let mut state = ViewportReadModelState::default();
    let a = SceneAnchor::active_session("/World/A");

    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(loaded_snapshot("standalone.usda", 0, vec![a.clone()], 1)),
        },
    ));
    state
        .begin_bim_catalogue_request("local-20".to_owned())
        .unwrap();
    let cat_old = make_catalogue(1, 3);
    state.apply(&ViewportEventEnvelope::new(
        Some("local-20".to_owned()),
        ViewportEvent::BimClassificationFieldCatalogueChanged {
            catalogue: cat_old.clone(),
        },
    ));
    assert_eq!(state.bim_classification_catalogue(), Some(&cat_old));

    // Pending property + catalogue requests before reload.
    state
        .begin_bim_properties_request("local-10".to_owned())
        .unwrap();
    state
        .begin_bim_catalogue_request("local-21".to_owned())
        .unwrap();

    // Stage reloads with same path and activation_generation == 0.
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::StageLoadStateChanged {
            state: StageLoadState::Loading,
        },
    ));
    assert_eq!(
        state.bim_properties_state(),
        BimPropertiesLoadState::StageNotLoaded
    );
    assert_eq!(
        state.bim_catalogue_state(),
        BimCatalogueLoadState::StageNotLoaded
    );

    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(loaded_snapshot("standalone.usda", 0, vec![a.clone()], 1)),
        },
    ));

    // Stale pre-reload responses with matching path/gen/revision must be rejected by stage_epoch.
    let props_old = make_bim_properties_model(
        vec![a.clone()],
        1,
        BimPropertyGroupId::Instance,
        "Instance",
        vec![("Mark", "Old")],
    );
    state.apply(&ViewportEventEnvelope::new(
        Some("local-10".to_owned()),
        ViewportEvent::BimPropertiesRead {
            properties: props_old,
            diff: None,
        },
    ));
    state.apply(&ViewportEventEnvelope::new(
        Some("local-21".to_owned()),
        ViewportEvent::BimClassificationFieldCatalogueChanged {
            catalogue: cat_old,
        },
    ));
    assert!(state.bim_properties().is_none());
    assert!(state.bim_classification_catalogue().is_none());

    // Load fresh data on reloaded stage, then switch directly to stage S2 (gen 2).
    state
        .begin_bim_catalogue_request("local-22".to_owned())
        .unwrap();
    state.apply(&ViewportEventEnvelope::new(
        Some("local-22".to_owned()),
        ViewportEvent::BimClassificationFieldCatalogueChanged {
            catalogue: make_catalogue(2, 2),
        },
    ));
    assert!(state.bim_classification_catalogue().is_some());

    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(loaded_snapshot("stage2.usda", 2, vec![a], 1)),
        },
    ));
    assert!(state.bim_properties().is_none());
    assert!(state.bim_classification_catalogue().is_none());
}

#[test]
fn bim_repeated_identical_snapshot_and_selection_delta_noop_preserve_pending_and_ready() {
    let mut state = ViewportReadModelState::default();
    let a = SceneAnchor::active_session("/World/A");
    let b = SceneAnchor::active_session("/World/B");
    let snap = loaded_snapshot("s.usda", 1, vec![a.clone()], 1);

    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(snap.clone()),
        },
    ));
    state
        .begin_bim_properties_request("local-11".to_owned())
        .unwrap();

    // Repeated identical snapshot during in-flight request preserves pending request.
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(snap),
        },
    ));
    let props_a = make_bim_properties_model(
        vec![a.clone()],
        1,
        BimPropertyGroupId::Instance,
        "Instance",
        vec![("Mark", "A1")],
    );
    state.apply(&ViewportEventEnvelope::new(
        Some("local-11".to_owned()),
        ViewportEvent::BimPropertiesRead {
            properties: props_a.clone(),
            diff: None,
        },
    ));
    assert_eq!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Ready(&props_a)
    );

    // No-op SelectionDeltaApplied preserves ready properties.
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::SelectionDeltaApplied {
            revision: 1,
            added: Vec::new(),
            removed: Vec::new(),
            primary: Some(a),
            count: 1,
        },
    ));
    assert_eq!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Ready(&props_a)
    );

    // Genuine SelectionDeltaApplied invalidates properties.
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::SelectionDeltaApplied {
            revision: 2,
            added: vec![b],
            removed: Vec::new(),
            primary: None,
            count: 2,
        },
    ));
    assert!(state.bim_properties().is_none());
    assert_eq!(state.bim_properties_state(), BimPropertiesLoadState::NotRequested);
}

#[test]
fn bim_transport_pages_assemble_out_of_order_idempotently_and_reject_corrupt_pages() {
    let mut state = ViewportReadModelState::default();
    let a = SceneAnchor::active_session("/World/A");
    let b = SceneAnchor::active_session("/World/B");

    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(loaded_snapshot("s.usda", 1, vec![a.clone()], 1)),
        },
    ));
    state
        .begin_bim_properties_request("local-30".to_owned())
        .unwrap();

    let page1 = viewport_protocol::BimPropertiesPage {
        selection_revision: 1,
        page_index: 1,
        page_count: 2,
        total_properties: 2,
        targets: Vec::new(),
        groups: vec![BimPropertyGroupReadModel {
            id: BimPropertyGroupId::Instance,
            name: "Instance".to_owned(),
            editable_group: false,
            properties: vec![make_bim_property("Prop2", "V2")],
        }],
        diff: None,
    };
    let page0 = viewport_protocol::BimPropertiesPage {
        selection_revision: 1,
        page_index: 0,
        page_count: 2,
        total_properties: 2,
        targets: vec![a.clone()],
        groups: vec![BimPropertyGroupReadModel {
            id: BimPropertyGroupId::Instance,
            name: "Instance".to_owned(),
            editable_group: false,
            properties: vec![make_bim_property("Prop1", "V1")],
        }],
        diff: None,
    };

    // Page 1 arrives before Page 0, then duplicate Page 1 -> incomplete Loading(1/2), not partial Ready.
    state.apply(&ViewportEventEnvelope::new(
        Some("local-30".to_owned()),
        ViewportEvent::BimPropertiesPage {
            page: page1.clone(),
        },
    ));
    state.apply(&ViewportEventEnvelope::new(
        Some("local-30".to_owned()),
        ViewportEvent::BimPropertiesPage { page: page1 },
    ));
    assert!(state.bim_properties().is_none());
    assert_eq!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Loading {
            received_pages: 1,
            total_pages: 2
        }
    );

    // Page 0 arrives -> merged in order [Prop1, Prop2].
    state.apply(&ViewportEventEnvelope::new(
        Some("local-30".to_owned()),
        ViewportEvent::BimPropertiesPage { page: page0 },
    ));
    let assembled = state.bim_properties().expect("assembled properties ready");
    assert_eq!(assembled.groups.len(), 1);
    let keys: Vec<&str> = assembled.groups[0]
        .properties
        .iter()
        .map(|p| p.key.as_str())
        .collect();
    assert_eq!(keys, vec!["Prop1", "Prop2"]);

    // Corrupt duplicate page -> explicit Error, never partial result.
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(loaded_snapshot("s.usda", 1, vec![b], 2)),
        },
    ));
    state
        .begin_bim_properties_request("local-31".to_owned())
        .unwrap();
    let bad_p0 = viewport_protocol::BimPropertiesPage {
        selection_revision: 2,
        page_index: 0,
        page_count: 2,
        total_properties: 2,
        targets: vec![SceneAnchor::active_session("/World/B")],
        groups: vec![BimPropertyGroupReadModel {
            id: BimPropertyGroupId::Instance,
            name: "Instance".to_owned(),
            editable_group: false,
            properties: vec![make_bim_property("PropA", "1")],
        }],
        diff: None,
    };
    let mut conflicting_p0 = bad_p0.clone();
    conflicting_p0.groups[0].properties[0] = make_bim_property("PropA", "2");
    state.apply(&ViewportEventEnvelope::new(
        Some("local-31".to_owned()),
        ViewportEvent::BimPropertiesPage { page: bad_p0 },
    ));
    state.apply(&ViewportEventEnvelope::new(
        Some("local-31".to_owned()),
        ViewportEvent::BimPropertiesPage {
            page: conflicting_p0,
        },
    ));
    assert!(state.bim_properties().is_none());
    assert!(matches!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Error(_)
    ));

    // Catalogue pages: out-of-order + idempotent duplicate -> Ready.
    state
        .begin_bim_catalogue_request("local-40".to_owned())
        .unwrap();
    let cat = make_catalogue(5, 3);
    let cat_p1 = viewport_protocol::BimClassificationFieldCataloguePage {
        semantic_revision: 5,
        page_index: 1,
        page_count: 2,
        total_fields: 3,
        fields: vec![cat.fields[2].clone()],
    };
    let cat_p0 = viewport_protocol::BimClassificationFieldCataloguePage {
        semantic_revision: 5,
        page_index: 0,
        page_count: 2,
        total_fields: 3,
        fields: vec![cat.fields[0].clone(), cat.fields[1].clone()],
    };
    state.apply(&ViewportEventEnvelope::new(
        Some("local-40".to_owned()),
        ViewportEvent::BimClassificationFieldCataloguePage {
            page: cat_p1.clone(),
        },
    ));
    state.apply(&ViewportEventEnvelope::new(
        Some("local-40".to_owned()),
        ViewportEvent::BimClassificationFieldCataloguePage { page: cat_p1 },
    ));
    assert_eq!(
        state.bim_catalogue_state(),
        BimCatalogueLoadState::Loading {
            received_pages: 1,
            total_pages: 2
        }
    );
    state.apply(&ViewportEventEnvelope::new(
        Some("local-40".to_owned()),
        ViewportEvent::BimClassificationFieldCataloguePage { page: cat_p0 },
    ));
    assert_eq!(state.bim_classification_catalogue(), Some(&cat));

    // Recovery A -> B -> A with new selection revision & request ID: old A response ignored.
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(loaded_snapshot("s.usda", 1, vec![a.clone()], 3)),
        },
    ));
    state
        .begin_bim_properties_request("local-32".to_owned())
        .unwrap();
    let stale_a = make_bim_properties_model(
        vec![a.clone()],
        1,
        BimPropertyGroupId::Instance,
        "Instance",
        vec![("Mark", "StaleA")],
    );
    let fresh_a = make_bim_properties_model(
        vec![a],
        3,
        BimPropertyGroupId::Instance,
        "Instance",
        vec![("Mark", "FreshA")],
    );
    state.apply(&ViewportEventEnvelope::new(
        Some("local-30".to_owned()),
        ViewportEvent::BimPropertiesRead {
            properties: stale_a,
            diff: None,
        },
    ));
    assert!(state.bim_properties().is_none());
    state.apply(&ViewportEventEnvelope::new(
        Some("local-32".to_owned()),
        ViewportEvent::BimPropertiesRead {
            properties: fresh_a.clone(),
            diff: None,
        },
    ));
    assert_eq!(state.bim_properties(), Some(&fresh_a));
}

#[test]
fn bim_properties_current_id_mismatched_metadata_transitions_to_error_and_recovers() {
    let mut state = ViewportReadModelState::default();
    let a = SceneAnchor::active_session("/World/A");
    let b = SceneAnchor::active_session("/World/B");

    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(loaded_snapshot("s.usda", 1, vec![a.clone()], 7)),
        },
    ));

    // 1. Current-ID BimPropertiesRead with wrong selection_revision -> Error (NOT Loading).
    state
        .begin_bim_properties_request("r7".to_owned())
        .unwrap();
    let wrong_rev = make_bim_properties_model(
        vec![a.clone()],
        8,
        BimPropertyGroupId::Instance,
        "Instance",
        vec![("Mark", "WrongRev")],
    );
    state.apply(&ViewportEventEnvelope::new(
        Some("r7".to_owned()),
        ViewportEvent::BimPropertiesRead {
            properties: wrong_rev,
            diff: None,
        },
    ));
    assert!(state.bim_properties().is_none());
    assert!(matches!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Error(_)
    ));

    // 2. Recoverable via a new request ID with matching metadata -> Ready.
    state
        .begin_bim_properties_request("r8".to_owned())
        .unwrap();
    let valid_a = make_bim_properties_model(
        vec![a.clone()],
        7,
        BimPropertyGroupId::Instance,
        "Instance",
        vec![("Mark", "ValidA")],
    );
    state.apply(&ViewportEventEnvelope::new(
        Some("r8".to_owned()),
        ViewportEvent::BimPropertiesRead {
            properties: valid_a.clone(),
            diff: None,
        },
    ));
    assert_eq!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Ready(&valid_a)
    );

    // 3. Current-ID BimPropertiesRead with wrong targets -> Error (NOT Loading).
    state
        .begin_bim_properties_request("r9".to_owned())
        .unwrap();
    let wrong_targets = make_bim_properties_model(
        vec![b],
        7,
        BimPropertyGroupId::Instance,
        "Instance",
        vec![("Mark", "WrongTargets")],
    );
    state.apply(&ViewportEventEnvelope::new(
        Some("r9".to_owned()),
        ViewportEvent::BimPropertiesRead {
            properties: wrong_targets,
            diff: None,
        },
    ));
    assert!(state.bim_properties().is_none());
    assert!(matches!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Error(_)
    ));

    // 4. Current-ID BimPropertiesError with wrong selection_revision -> Error (NOT Loading).
    state
        .begin_bim_properties_request("r10".to_owned())
        .unwrap();
    state.apply(&ViewportEventEnvelope::new(
        Some("r10".to_owned()),
        ViewportEvent::BimPropertiesError {
            error: viewport_protocol::BimPropertiesDeliveryError {
                selection_revision: 8,
                kind: viewport_protocol::BimPropertiesDeliveryErrorKind::OversizedMetadata,
                property: None,
                encoded_bytes: 2048,
                max_bytes: 1024,
            },
        },
    ));
    assert!(matches!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Error(_)
    ));

    // 5. Stale/unrelated request_id mismatched responses must NOT fail active pending request.
    state
        .begin_bim_properties_request("r11".to_owned())
        .unwrap();
    let stale_wrong = make_bim_properties_model(
        vec![a],
        99,
        BimPropertyGroupId::Instance,
        "Instance",
        vec![("Mark", "Stale")],
    );
    state.apply(&ViewportEventEnvelope::new(
        Some("r7".to_owned()),
        ViewportEvent::BimPropertiesRead {
            properties: stale_wrong,
            diff: None,
        },
    ));
    state.apply(&ViewportEventEnvelope::new(
        Some("r10".to_owned()),
        ViewportEvent::BimPropertiesError {
            error: viewport_protocol::BimPropertiesDeliveryError {
                selection_revision: 99,
                kind: viewport_protocol::BimPropertiesDeliveryErrorKind::TooManyProperties,
                property: None,
                encoded_bytes: 1024,
                max_bytes: 512,
            },
        },
    ));
    assert_eq!(
        state.bim_properties_state(),
        BimPropertiesLoadState::Loading {
            received_pages: 0,
            total_pages: 0
        }
    );
}

#[test]
fn bim_catalogue_anonymous_events_never_install_directly_and_coalesce_refresh() {
    let mut state = ViewportReadModelState::default();
    let a = SceneAnchor::active_session("/World/A");

    // Stage A loaded and populated via correlated request Q1 at revision 12.
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(loaded_snapshot("stage_a.usda", 1, vec![a.clone()], 1)),
        },
    ));
    let epoch_a = state.stage_epoch();
    state
        .begin_bim_catalogue_request("Q1".to_owned())
        .unwrap();
    let cat_a12 = make_catalogue(12, 3);
    state.apply(&ViewportEventEnvelope::new(
        Some("Q1".to_owned()),
        ViewportEvent::BimClassificationFieldCatalogueChanged {
            catalogue: cat_a12.clone(),
        },
    ));
    assert_eq!(state.bim_classification_catalogue(), Some(&cat_a12));
    assert_eq!(state.pending_catalogue_refresh(), None);

    // Same-stage anonymous revision rollback (rev 11, request_id: None) -> never installs rev 11,
    // clears stale cached catalogue, and records coalesced refresh marker.
    let cat_a11 = make_catalogue(11, 2);
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::BimClassificationFieldCatalogueChanged {
            catalogue: cat_a11.clone(),
        },
    ));
    assert!(state.bim_classification_catalogue().is_none());
    assert_eq!(state.bim_catalogue_state(), BimCatalogueLoadState::NotRequested);
    assert_eq!(state.pending_catalogue_refresh(), Some((epoch_a, 1)));

    // Burst of anonymous Changed & Page events coalesces to a single marker without installing data.
    for rev in 1..=50 {
        state.apply(&ViewportEventEnvelope::new(
            None,
            ViewportEvent::BimClassificationFieldCatalogueChanged {
                catalogue: make_catalogue(rev, 1),
            },
        ));
        state.apply(&ViewportEventEnvelope::new(
            None,
            ViewportEvent::BimClassificationFieldCataloguePage {
                page: viewport_protocol::BimClassificationFieldCataloguePage {
                    semantic_revision: rev,
                    page_index: 0,
                    page_count: 1,
                    total_fields: 1,
                    fields: cat_a11.fields[..1].to_vec(),
                },
            },
        ));
    }
    assert!(state.bim_classification_catalogue().is_none());
    assert_eq!(state.pending_catalogue_refresh(), Some((epoch_a, 1)));
    assert!(state.take_catalogue_refresh_for_current_stage());
    assert!(!state.take_catalogue_refresh_for_current_stage());

    // Stage A -> B transition clears prior marker; late anonymous A payload never installs under B.
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(loaded_snapshot("stage_b.usda", 2, vec![a.clone()], 1)),
        },
    ));
    let epoch_b = state.stage_epoch();
    assert_ne!(epoch_a, epoch_b);
    assert_eq!(state.pending_catalogue_refresh(), None);

    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::BimClassificationFieldCatalogueChanged {
            catalogue: cat_a12,
        },
    ));
    assert!(state.bim_classification_catalogue().is_none());
    assert_eq!(state.pending_catalogue_refresh(), Some((epoch_b, 2)));

    // While correlated Q2 is pending, anonymous signals are ignored cleanly without clobbering Q2.
    state
        .begin_bim_catalogue_request("Q2".to_owned())
        .unwrap();
    assert_eq!(state.pending_catalogue_refresh(), None);
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::BimClassificationFieldCatalogueChanged {
            catalogue: cat_a11,
        },
    ));
    assert_eq!(
        state.bim_catalogue_state(),
        BimCatalogueLoadState::Loading {
            received_pages: 0,
            total_pages: 0
        }
    );
    let cat_b = make_catalogue(20, 2);
    state.apply(&ViewportEventEnvelope::new(
        Some("Q2".to_owned()),
        ViewportEvent::BimClassificationFieldCatalogueChanged {
            catalogue: cat_b.clone(),
        },
    ));
    assert_eq!(state.bim_classification_catalogue(), Some(&cat_b));
    assert_eq!(state.pending_catalogue_refresh(), None);

    // Non-Ready stage ignores anonymous catalogue events and never sets refresh marker.
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::StageLoadStateChanged {
            state: StageLoadState::Loading,
        },
    ));
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::BimClassificationFieldCatalogueChanged {
            catalogue: make_catalogue(99, 1),
        },
    ));
    assert_eq!(
        state.bim_catalogue_state(),
        BimCatalogueLoadState::StageNotLoaded
    );
    assert_eq!(state.pending_catalogue_refresh(), None);

    // Same-path reload with activation_generation == 0 isolates epoch cleanly.
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::Snapshot {
            state: Box::new(loaded_snapshot("stage_b.usda", 0, vec![a], 1)),
        },
    ));
    state.apply(&ViewportEventEnvelope::new(
        None,
        ViewportEvent::BimClassificationFieldCatalogueChanged {
            catalogue: make_catalogue(1, 1),
        },
    ));
    assert!(state.bim_classification_catalogue().is_none());
    assert_eq!(
        state.pending_catalogue_refresh(),
        Some((state.stage_epoch(), 0))
    );
}
