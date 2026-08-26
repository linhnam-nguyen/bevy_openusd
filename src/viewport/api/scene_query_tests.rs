use super::*;

fn node(
    path: &str,
    parent: Option<&str>,
    name: &str,
    usd_display_name: Option<&str>,
) -> viewport_protocol::PrimNodeReadModel {
    viewport_protocol::PrimNodeReadModel {
        anchor: SceneAnchor::active_session(path),
        parent: parent.map(SceneAnchor::active_session),
        label: name.to_owned(),
        display_name: usd_display_name.map(str::to_owned),
        visible: true,
        has_children: false,
    }
}

fn hierarchy(nodes: &[viewport_protocol::PrimNodeReadModel]) -> HierarchyReadModel {
    HierarchyReadModel::from_prim_nodes(nodes)
}

#[test]
fn search_returns_ancestor_pages_for_reveal() {
    let nodes = vec![
        node("/World", None, "World", None),
        node("/World/Environment", Some("/World"), "Environment", None),
        node(
            "/World/Environment/Door",
            Some("/World/Environment"),
            "Door",
            None,
        ),
    ];

    let (total, matches) = search_hierarchy(&hierarchy(&nodes), "door", 0, 10);

    assert_eq!(total, 1);
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0].anchor.as_ref().unwrap().prim_path,
        "/World/Environment/Door"
    );
    assert_eq!(
        matches[0]
            .reveal_pages
            .iter()
            .map(|page| page.parent.as_ref().map(|parent| parent.prim_path.as_str()))
            .collect::<Vec<_>>(),
        vec![None, Some("/World"), Some("/World/Environment")]
    );
}

#[test]
fn search_matches_projected_names_only_and_preserves_context() {
    let nodes = vec![
        node("/root", None, "root", None),
        node("/root/name1", Some("/root"), "name1", None),
        node("/root/name1/name2", Some("/root/name1"), "name2", None),
        node(
            "/root/name1/name2/name3",
            Some("/root/name1/name2"),
            "name3",
            None,
        ),
        node("/root/name10", Some("/root"), "name10", None),
        node("/root/name10/name20", Some("/root/name10"), "name20", None),
        node(
            "/Building/Level01/Wall_0042",
            None,
            "Wall_0042",
            Some("Exterior Wall"),
        ),
    ];
    let hierarchy = hierarchy(&nodes);

    let assert_one = |query: &str, path: &str, name: &str| {
        let (total, matches) = search_hierarchy(&hierarchy, query, 0, 10);
        assert_eq!(total, 1, "query {query}");
        assert_eq!(matches.len(), 1, "query {query}");
        assert_eq!(matches[0].name, name);
        assert_eq!(matches[0].breadcrumb, path);
        assert_eq!(matches[0].prim_path.as_deref(), Some(path));
    };

    assert_one("name2", "/root/name1/name2", "name2");
    assert_one("name3", "/root/name1/name2/name3", "name3");
    assert_one("name1", "/root/name1", "name1");
    assert_one("Wall_0042", "/Building/Level01/Wall_0042", "Wall_0042");
    assert_eq!(search_hierarchy(&hierarchy, "missing", 0, 10).0, 0);
    assert_eq!(search_hierarchy(&hierarchy, "Exterior Wall", 0, 10).0, 0);
}

#[test]
fn search_is_decoupled_from_prim_paths_for_synthetic_projections() {
    let category = HierarchyNode {
        id: HierarchyNodeId("category-a".to_owned()),
        parent_id: None,
        name: "Category A".to_owned(),
        breadcrumb: "Category A".to_owned(),
        prim_path: None,
        anchor: None,
        parent_anchor: None,
        visible: true,
        has_children: true,
    };
    let group = HierarchyNode {
        id: HierarchyNodeId("group-b".to_owned()),
        parent_id: Some(category.id.clone()),
        name: "Group B".to_owned(),
        breadcrumb: "Category A/Group B".to_owned(),
        prim_path: None,
        anchor: None,
        parent_anchor: None,
        visible: true,
        has_children: true,
    };
    let object = HierarchyNode {
        id: HierarchyNodeId("object-c".to_owned()),
        parent_id: Some(group.id.clone()),
        name: "Object C".to_owned(),
        breadcrumb: "Category A/Group B/Object C".to_owned(),
        prim_path: Some("/Completely/Different/Usd/Path".to_owned()),
        anchor: None,
        parent_anchor: None,
        visible: true,
        has_children: false,
    };
    let hierarchy = HierarchyReadModel {
        nodes: vec![category, group, object],
    };

    let (total, matches) = search_hierarchy(&hierarchy, "Object C", 0, 10);
    assert_eq!(total, 1);
    assert_eq!(matches[0].name, "Object C");
    assert_eq!(matches[0].breadcrumb, "Category A/Group B/Object C");
    assert_eq!(
        matches[0].prim_path.as_deref(),
        Some("/Completely/Different/Usd/Path")
    );
    assert_eq!(search_hierarchy(&hierarchy, "Different", 0, 10).0, 0);
}

#[test]
fn search_paginates_exact_projected_names_without_path_matching() {
    let nodes = vec![
        node("/World/Left/Door", Some("/World/Left"), "Door", None),
        node("/World/Right/Door", Some("/World/Right"), "Door", None),
        node("/World/DoorPanel", Some("/World"), "DoorPanel", None),
    ];
    let hierarchy = hierarchy(&nodes);

    let (total, first) = search_hierarchy(&hierarchy, "door", 0, 1);
    assert_eq!(total, 2);
    assert_eq!(first[0].breadcrumb, "/World/Left/Door");

    let (_, second) = search_hierarchy(&hierarchy, "door", 1, 1);
    assert_eq!(second[0].breadcrumb, "/World/Right/Door");
    assert_eq!(search_hierarchy(&hierarchy, "doorpanel", 0, 10).0, 1);
    assert_eq!(search_hierarchy(&hierarchy, "World", 0, 10).0, 0);
}
