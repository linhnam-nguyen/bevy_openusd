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
