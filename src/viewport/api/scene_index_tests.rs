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
    let index = SceneAnchorIndex::from_test_nodes(vec![
        node("/World", None, "World"),
        node("/World/Environment", Some("/World"), "Environment"),
        node(
            "/World/Environment/Door",
            Some("/World/Environment"),
            "Door",
        ),
    ]);

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
fn indexed_hierarchy_pages_and_search_preserve_order_without_full_scans() {
    let index = SceneAnchorIndex::from_test_nodes(vec![
        node("/World", None, "World"),
        node("/World/A", Some("/World"), "A"),
        node("/World/B", Some("/World"), "B"),
        node("/World/C", Some("/World"), "C"),
    ]);

    let page = index.children_page(Some(&SceneAnchor::active_session("/World")), 1, 2);
    assert_eq!(page.total, 3);
    assert_eq!(page.nodes.len(), 1);
    assert_eq!(page.nodes[0].anchor.prim_path, "/World/C");
    assert_eq!(index.page_by_anchor.len(), 4);

    let result = index
        .search_match_for_path("/World/C")
        .expect("indexed path resolves");
    assert_eq!(result.label, "C");
    assert_eq!(result.reveal_pages.len(), 2);
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
        Option<&UsdHierarchyTarget>,
        Option<&UsdTransparentHierarchyNode>,
        Option<&Visibility>,
        Option<&Children>,
    )>();
    let prims = prims.query(&world);
    index.rebuild(&prims, None);

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

#[test]
fn canonical_project_activation_projects_manifest_names_into_protocol_read_model() {
    use crate::viewport::session::{Spawned, StagePresentationContext};
    use bevy::app::App;
    use openusd::usd::Stage;
    use project_protocol::ProjectStageTarget;
    use tempfile::tempdir;
    use usd_bevy::{LiveStage, LiveStagePlugin, UsdPlugin};
    use usd_project::ProjectRoot;

    let directory = tempdir().expect("temporary Project directory");
    let projects = directory.path().join("projects");
    std::fs::create_dir(&projects).expect("Project parent directory");
    let mut service = crate::project::service::ProjectApplicationService::open(
        directory.path().join("workspace.json"),
    )
    .expect("Project service opens");
    let project = service
        .create_project(&projects, "Pro3")
        .expect("Project creates");
    let root_scene_id = match project.root.clone() {
        ProjectRoot::Scene(scene_id) => scene_id,
        _ => panic!("new Project has a canonical Scene root"),
    };
    let child = service
        .create_scene(
            project.id,
            project_protocol::ProjectWriteTarget::Scene(root_scene_id),
            "Kitchen_set",
        )
        .expect("nested Scene creates");
    let target = service
        .resolve_stage_activation(
            project.id,
            ProjectStageTarget::ProjectRoot(ProjectRoot::Scene(root_scene_id)),
        )
        .expect("Project root resolves")
        .expect("Project has a Scene root");
    let stage = Stage::open(target.path.to_string_lossy().as_ref()).expect("canonical Stage opens");

    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<SceneAnchorIndex>()
        .add_systems(
            bevy::app::Update,
            refresh_scene_anchor_index.after(usd_bevy::LiveStageSet::Reconcile),
        );
    app.world_mut().insert_resource(Spawned(true));
    app.world_mut()
        .insert_resource(StagePresentationContext::from_project(target.presentation));
    app.world_mut().insert_non_send(LiveStage::new(stage));

    for _ in 0..8 {
        app.update();
    }

    let read_model = app
        .world()
        .resource::<SceneAnchorIndex>()
        .roots_read_model();
    let root_node = read_model
        .prims
        .iter()
        .find(|node| node.anchor.prim_path == "/SceneRoot")
        .expect("canonical Scene root is in the protocol read model");
    assert_eq!(root_node.display_name.as_deref(), Some("Pro3"));
    let members = app.world().resource::<SceneAnchorIndex>().children_page(
        Some(&SceneAnchor::active_session("/SceneRoot")),
        0,
        100,
    );
    let child_node = members
        .nodes
        .iter()
        .find(|node| node.display_name.as_deref() == Some("Kitchen_set"))
        .expect("managed member resolves to the manifest Scene display name");
    assert!(
        child_node
            .anchor
            .prim_path
            .contains(&child.scene_id.to_string())
            || child_node
                .anchor
                .prim_path
                .starts_with("/SceneRoot/Member_")
    );
    assert!(
        app.world()
            .resource::<SceneAnchorIndex>()
            .nodes
            .iter()
            .all(|node| !node
                .display_name
                .as_deref()
                .is_some_and(|name| name.starts_with("Member_")))
    );
    assert!(
        app.world()
            .resource::<SceneAnchorIndex>()
            .hierarchy_snapshot()
            .nodes
            .iter()
            .all(|node| !node.name.starts_with("Member_"))
    );
    assert_eq!(
        app.world()
            .resource::<SceneAnchorIndex>()
            .hierarchy_snapshot()
            .nodes
            .iter()
            .find(|node| node.prim_path.as_deref() == Some("/SceneRoot"))
            .map(|node| node.name.as_str()),
        Some("Pro3")
    );
}
