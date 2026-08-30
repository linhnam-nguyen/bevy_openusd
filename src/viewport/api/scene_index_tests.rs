use super::*;
use crate::viewport::api::HierarchyPageIndex;
use crate::viewport::session::Spawned;
use bevy::asset::Assets;
use bevy::mesh::Mesh;
use bevy::pbr::StandardMaterial;
use usd_bevy::{LiveStage, LiveStagePlugin, LiveStageSet, UsdPlugin};

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
    let projection =
        CurrentHierarchyProjection::from_prim_nodes(&[node("/World", None, "World")], 1);

    let first = projection.snapshot();
    let second = projection.snapshot();

    assert!(std::sync::Arc::ptr_eq(&first, &second));
}

#[test]
fn shared_provider_projection_installs_arc_and_matching_page_index() {
    let read_model = std::sync::Arc::new(viewport_protocol::HierarchyReadModel {
        source: viewport_protocol::HierarchySource::BimClassification,
        revision: 3,
        nodes: Vec::new(),
    });
    let page_index = HierarchyPageIndex::from_read_model(&read_model);
    let projection = CurrentHierarchyProjection::from_shared_parts(
        std::sync::Arc::clone(&read_model),
        page_index,
    );

    assert!(std::sync::Arc::ptr_eq(&read_model, &projection.snapshot()));
    assert_eq!(projection.children_page(None, 0, 1).unwrap().total, 0);
}

#[test]
fn generic_projection_keeps_snapshot_acquisition_constant_time() {
    let nodes: Vec<PrimNodeReadModel> = (0..2_000)
        .map(|index| {
            let path = format!("/World/Element{index:04}");
            let label = format!("Element {index:04}");
            node(&path, None, &label)
        })
        .collect();

    let projection_started = std::time::Instant::now();
    let projection = CurrentHierarchyProjection::from_prim_nodes(&nodes, 7);
    let projection_elapsed = projection_started.elapsed();

    let snapshot_started = std::time::Instant::now();
    let first = projection.snapshot();
    let snapshot_elapsed = snapshot_started.elapsed();
    let second = projection.snapshot();

    let roots = projection
        .children_page(None, 0, 1_000)
        .expect("root page is valid");
    assert_eq!(roots.total, 2_000);
    assert_eq!(roots.nodes.len(), MAX_SCENE_PAGE_SIZE as usize);
    assert!(roots.has_more);
    assert!(std::sync::Arc::ptr_eq(&first, &second));
    assert_eq!(first.revision, 7);

    eprintln!(
        "M2-C7 generic projection: nodes={} roots={} projection_ms={:.3} snapshot_us={:.3}",
        nodes.len(),
        roots.total,
        projection_elapsed.as_secs_f64() * 1_000.0,
        snapshot_elapsed.as_secs_f64() * 1_000_000.0,
    );
}

fn native_instance_scene_index_app() -> App {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/stages/native_instance_characterization.usda");
    let stage = openusd::usd::Stage::open(path.to_str().expect("fixture path is valid"))
        .expect("native instance fixture opens");
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<CurrentHierarchyProjection>()
        .init_resource::<Spawned>()
        .add_systems(
            Update,
            refresh_scene_anchor_index.after(LiveStageSet::Reconcile),
        );
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.world_mut().resource_mut::<Spawned>().0 = true;
    app.update();
    app
}

#[test]
fn native_instance_selection_resolves_real_projected_scene_proxy_paths_only() {
    let app = native_instance_scene_index_app();
    let index = app.world().resource::<SceneAnchorIndex>();
    let frame_a = index
        .resolve(&SceneAnchor::active_session("/World/Window_A/Frame"))
        .expect("Window_A frame is indexed from the native projection");
    let frame_b = index
        .resolve(&SceneAnchor::active_session("/World/Window_B/Frame"))
        .expect("Window_B frame is indexed from the native projection");
    assert_ne!(frame_a, frame_b);

    assert_eq!(
        index.anchor_for(frame_a).unwrap().prim_path,
        "/World/Window_A/Frame"
    );
    assert_eq!(
        index.resolve(&SceneAnchor::active_session("/__Prototype_1/Frame")),
        None,
        "prototype paths are not selectable scene identities"
    );
}
