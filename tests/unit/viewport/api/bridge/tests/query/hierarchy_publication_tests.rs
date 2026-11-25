use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::*;
use viewport_protocol::{
    HierarchySource, PrimNodeReadModel, SceneAnchor, StageLoadState, ViewportCommand,
    ViewportCommandEnvelope, ViewportEvent,
};

use crate::api::bridge::query::{dispatch_scene_query_commands, publish_stage_load_state};
use crate::api::bridge::state::SceneSearchRequests;
use crate::api::query::SceneQueryService;
use crate::api::{ViewportCommandInbox, ViewportEventOutbox};
use crate::viewport::rendering::scene::{
    ActiveHierarchyProvider, CurrentHierarchyProjection, SceneAnchorIndex,
    refresh_scene_anchor_index, register_scene_index_observers,
};
use crate::viewport::stage::{Spawned, StageHandle, StageInfo};

use super::support::command_test_app;

#[test]
fn ready_publication_waits_for_final_derived_flush_and_publishes_consistent_tree() {
    let mut app = command_test_app();
    app.init_resource::<ActiveHierarchyProvider>()
        .init_resource::<SceneQueryService>()
        .init_resource::<SceneSearchRequests>()
        .insert_resource(StageHandle {
            path: "fixtures/scene.usda".into(),
            error: None,
        })
        .insert_resource(StageInfo {
            activation_generation: 2,
            path: "fixtures/scene.usda".to_owned(),
            ..Default::default()
        })
        .add_systems(
            Update,
            (
                refresh_scene_anchor_index,
                dispatch_scene_query_commands,
                publish_stage_load_state,
            )
                .chain(),
        );
    register_scene_index_observers(&mut app);

    let root = app
        .world_mut()
        .spawn(usd_bevy::UsdPrimRef::new("/SceneRoot"))
        .id();
    app.world_mut()
        .spawn((usd_bevy::UsdPrimRef::new("/SceneRoot/Wall"), ChildOf(root)));
    app.world_mut().resource_mut::<Spawned>().0 = true;

    // Frame 1: bounded admission ran, derived flush has not run yet.
    // Lifecycle must not publish a premature empty Ready snapshot.
    app.update();
    let frame1_events = app.world_mut().resource_mut::<ViewportEventOutbox>().take_published();
    assert!(
        !frame1_events.iter().any(|envelope| matches!(
            &envelope.event,
            ViewportEvent::StageLoadStateChanged {
                state: StageLoadState::Ready
            }
        )),
        "Ready must not be published before initial derived flush installs visible hierarchy"
    );

    // Frame 2: derived flush runs, advances published_revision, and Ready
    // snapshot carries the populated root with accurate has_children metadata.
    app.update();
    let frame2_events = app.world_mut().resource_mut::<ViewportEventOutbox>().take_published();
    assert!(frame2_events.iter().any(|envelope| matches!(
        &envelope.event,
        ViewportEvent::StageLoadStateChanged {
            state: StageLoadState::Ready
        }
    )));
    let snapshot = frame2_events
        .iter()
        .find_map(|envelope| match &envelope.event {
            ViewportEvent::Snapshot { state } => Some(state.as_ref()),
            _ => None,
        })
        .expect("Ready transition must emit a populated snapshot");
    assert!(snapshot.stage.loaded);
    assert_eq!(snapshot.stage.activation_generation, 2);
    assert_eq!(snapshot.scene.total_prims, 2);
    assert_eq!(snapshot.scene.prims.len(), 1);
    assert_eq!(snapshot.scene.prims[0].anchor.prim_path, "/SceneRoot");
    assert!(snapshot.scene.prims[0].has_children);

    // Frame 3 (idle): no redundant snapshot emitted when nothing changed.
    app.update();
    assert!(
        app.world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .take_published()
            .is_empty()
    );
}

#[test]
fn snapshot_roots_and_child_pages_read_from_the_same_published_projection() {
    let mut app = command_test_app();
    app.init_resource::<ActiveHierarchyProvider>()
        .init_resource::<SceneQueryService>()
        .init_resource::<SceneSearchRequests>()
        .add_systems(Update, dispatch_scene_query_commands);

    let root_anchor = SceneAnchor::active_session("/SceneRoot");
    let child_anchor = SceneAnchor::active_session("/SceneRoot/Member_01");
    let cached_nodes = vec![
        PrimNodeReadModel {
            anchor: root_anchor.clone(),
            parent: None,
            label: "SceneRoot".to_owned(),
            display_name: Some("Pro3".to_owned()),
            visible: true,
            has_children: true,
        },
        PrimNodeReadModel {
            anchor: child_anchor.clone(),
            parent: Some(root_anchor.clone()),
            label: "Member_01".to_owned(),
            display_name: Some("Sc1".to_owned()),
            visible: true,
            has_children: true,
        },
    ];
    // Simulate cache-backed CurrentHierarchyProjection installed while
    // SceneAnchorIndex is still empty (before live ECS entities project).
    *app.world_mut().resource_mut::<CurrentHierarchyProjection>() =
        CurrentHierarchyProjection::from_prim_nodes(&cached_nodes, 5);
    assert_eq!(
        app.world()
            .resource::<SceneAnchorIndex>()
            .roots_read_model()
            .total_prims,
        0
    );

    app.world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .push(ViewportCommandEnvelope::new(
            "snap-1",
            ViewportCommand::RequestSnapshot,
        ));
    app.world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .push(ViewportCommandEnvelope::new(
            "scene-children-1",
            ViewportCommand::RequestSceneChildren {
                parent: Some(root_anchor.clone()),
                page: 0,
                page_size: 16,
            },
        ));
    app.world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .push(ViewportCommandEnvelope::new(
            "hierarchy-children-1",
            ViewportCommand::RequestHierarchyChildren {
                source: HierarchySource::Prim,
                parent_id: Some(viewport_protocol::HierarchyNodeId::for_scene_anchor(
                    &root_anchor,
                )),
                page: 0,
                page_size: 16,
            },
        ));

    app.update();
    let events = app.world_mut().resource_mut::<ViewportEventOutbox>().take_published();

    let snapshot = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            ViewportEvent::Snapshot { state } => Some(state.as_ref()),
            _ => None,
        })
        .expect("RequestSnapshot emits unified root read model");
    assert_eq!(snapshot.scene.total_prims, 2);
    assert_eq!(snapshot.scene.prims.len(), 1);
    assert_eq!(snapshot.scene.prims[0].display_name.as_deref(), Some("Pro3"));

    let scene_children = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            ViewportEvent::SceneChildren { page } => Some(page),
            _ => None,
        })
        .expect("RequestSceneChildren reads from published projection");
    assert_eq!(scene_children.nodes.len(), 1);
    assert_eq!(scene_children.nodes[0].display_name.as_deref(), Some("Sc1"));

    let hierarchy_children = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            ViewportEvent::HierarchyChildren { page, .. } => Some(page),
            _ => None,
        })
        .expect("RequestHierarchyChildren reads from same published projection");
    assert_eq!(hierarchy_children.nodes.len(), 1);
    assert_eq!(hierarchy_children.nodes[0].name, "Sc1");
}
