use super::*;

fn node(path: &str, parent: Option<&str>, label: &str) -> PrimNodeReadModel {
    PrimNodeReadModel {
        anchor: SceneAnchor::active_session(path),
        parent: parent.map(SceneAnchor::active_session),
        label: label.to_owned(),
        display_name: Some(label.to_owned()),
        visible: true,
        has_children: false,
    }
}

#[test]
fn semantic_path_resolution_preserves_runtime_reveal_pages() {
    let index = SceneAnchorIndex {
        nodes: vec![
            node("/World", None, "World"),
            node("/World/Environment", Some("/World"), "Environment"),
            node(
                "/World/Environment/Door",
                Some("/World/Environment"),
                "Door",
            ),
        ],
        ..Default::default()
    };

    let result = index
        .search_match_for_path("/World/Environment/Door")
        .expect("semantic row resolves to a runtime node");
    assert_eq!(result.label, "Door");
    assert_eq!(result.reveal_pages.len(), 3);
    assert_eq!(result.reveal_pages[0].parent, None);
    assert_eq!(
        result.reveal_pages[1]
            .parent
            .as_ref()
            .map(|anchor| anchor.prim_path.as_str()),
        Some("/World")
    );
    assert_eq!(
        result.reveal_pages[2]
            .parent
            .as_ref()
            .map(|anchor| anchor.prim_path.as_str()),
        Some("/World/Environment")
    );
}

#[test]
fn prim_name_projection_uses_only_the_final_path_segment() {
    assert_eq!(prim_name("/root/name1/name2/name3"), "name3");
    assert_eq!(prim_name("/Architecture/Level01/Wall_0042"), "Wall_0042");
}

#[test]
fn hierarchy_snapshot_reuses_cached_projection() {
    let index = SceneAnchorIndex::from_test_nodes(vec![node("/World", None, "World")]);

    let first = index.hierarchy_snapshot();
    let second = index.hierarchy_snapshot();

    assert!(std::sync::Arc::ptr_eq(&first, &second));
}

#[test]
fn explicit_source_role_flattens_only_usdhub_wrappers_and_preserves_anchors() {
    let mut world = World::new();
    let stage_root = world.spawn(UsdPrimRef::new("/")).id();
    let scene_root = world
        .spawn((
            UsdPrimRef::new("/SceneRoot"),
            UsdDisplayName("Pro2".to_owned()),
            ChildOf(stage_root),
        ))
        .id();
    let member = world
        .spawn((
            UsdPrimRef::new("/SceneRoot/Member_member"),
            UsdDisplayName("Lv1".to_owned()),
            ChildOf(scene_root),
        ))
        .id();
    let wrapped_source = world
        .spawn((
            UsdPrimRef::new("/SceneRoot/Member_member/Source"),
            UsdTransparentHierarchyNode,
            ChildOf(member),
        ))
        .id();
    world.spawn((
        UsdPrimRef::new("/SceneRoot/Member_member/SourceAsset"),
        ChildOf(wrapped_source),
    ));
    world.spawn((UsdPrimRef::new("/SceneRoot/Members"), ChildOf(scene_root)));
    world.spawn((UsdPrimRef::new("/SceneRoot/Source"), ChildOf(scene_root)));

    let mut index = SceneAnchorIndex::default();
    let mut prims = world.query::<(
        Entity,
        &UsdPrimRef,
        Option<&UsdDisplayName>,
        Option<&UsdTransparentHierarchyNode>,
        Option<&Visibility>,
        Option<&Children>,
    )>();
    let prims = prims.query(&world);
    index.rebuild(&prims);

    let mut paths: Vec<_> = index
        .nodes
        .iter()
        .map(|node| node.anchor.prim_path.as_str())
        .collect();
    paths.sort_unstable();
    assert_eq!(
        paths,
        vec![
            "/SceneRoot",
            "/SceneRoot/Member_member",
            "/SceneRoot/Member_member/SourceAsset",
            "/SceneRoot/Members",
            "/SceneRoot/Source",
        ]
    );
    let member_node = index
        .nodes
        .iter()
        .find(|node| node.anchor.prim_path == "/SceneRoot/Member_member")
        .expect("managed member remains in the hierarchy");
    assert_eq!(member_node.display_name.as_deref(), Some("Lv1"));
    assert!(member_node.has_children);
    let source_asset = index
        .nodes
        .iter()
        .find(|node| node.anchor.prim_path.ends_with("/SourceAsset"))
        .expect("wrapped source content remains visible");
    assert_eq!(
        source_asset
            .parent
            .as_ref()
            .map(|anchor| anchor.prim_path.as_str()),
        Some("/SceneRoot/Member_member")
    );
    assert_eq!(
        index
            .nodes
            .iter()
            .find(|node| node.anchor.prim_path == "/SceneRoot")
            .and_then(|node| node.display_name.as_deref()),
        Some("Pro2")
    );
    assert!(
        index
            .resolve(&SceneAnchor::active_session(
                "/SceneRoot/Member_member/Source"
            ))
            .is_some(),
        "physical wrapper anchor remains selectable even when omitted from the tree"
    );
}
