use bevy::prelude::*;
use viewport_protocol::{
    HierarchyNodeId, HierarchyNodeKind, HierarchyNodeReadModel, HierarchyReadModel,
    HierarchySource, HierarchyVisibilityState, SceneAnchor, ViewportCommand, ViewportEvent,
};

use super::support::command_test_app;
use crate::viewport::api::bridge::commands::apply_viewport_commands;
use crate::viewport::api::bridge::tree::apply_tree_commands;
use crate::viewport::api::{
    CurrentHierarchyProjection, HierarchyPageIndex, HierarchyVisibilityIndex,
    HierarchyVisibilityTarget, SceneAnchorIndex, ViewportCommandInbox, ViewportEventOutbox,
};

#[test]
fn bim_group_visibility_updates_all_scene_occurrences_and_publishes_authoritative_event() {
    let mut app = command_test_app();
    app.add_systems(Update, apply_tree_commands.after(apply_viewport_commands));

    let first = app.world_mut().spawn(Visibility::Visible).id();
    let second = app.world_mut().spawn(Visibility::Visible).id();
    let first_anchor = SceneAnchor::active_session("/World/Window_A");
    let second_anchor = SceneAnchor::active_session("/World/Window_B");
    *app.world_mut().resource_mut::<SceneAnchorIndex>() =
        SceneAnchorIndex::from_test_entities(vec![
            (first_anchor.clone(), first),
            (second_anchor.clone(), second),
        ]);

    let group_id = HierarchyNodeId::new("bim-group-windows");
    let first_id = HierarchyNodeId::new("bim-leaf-window-a");
    let second_id = HierarchyNodeId::new("bim-leaf-window-b");
    let read_model = HierarchyReadModel {
        source: HierarchySource::BimClassification,
        revision: 1,
        nodes: vec![
            HierarchyNodeReadModel::virtual_node_with_kind(
                group_id.clone(),
                None,
                "Fenêtres".to_owned(),
                "Fenêtres".to_owned(),
                HierarchyNodeKind::Group,
                false,
                true,
            ),
            HierarchyNodeReadModel::scene(
                first_id.clone(),
                Some(group_id.clone()),
                "Window A".to_owned(),
                "Fenêtres / Window A".to_owned(),
                first_anchor.clone(),
                None,
                true,
                false,
            ),
            HierarchyNodeReadModel::scene(
                second_id.clone(),
                Some(group_id.clone()),
                "Window B".to_owned(),
                "Fenêtres / Window B".to_owned(),
                second_anchor.clone(),
                None,
                true,
                false,
            ),
        ],
    };
    let visibility_index =
        HierarchyVisibilityIndex::from_targets(std::collections::HashMap::from([
            (
                group_id.clone(),
                vec![
                    HierarchyVisibilityTarget::PrimPath(first_anchor.prim_path.clone()),
                    HierarchyVisibilityTarget::PrimPath(second_anchor.prim_path.clone()),
                ],
            ),
            (
                first_id.clone(),
                vec![HierarchyVisibilityTarget::PrimPath(
                    first_anchor.prim_path.clone(),
                )],
            ),
            (
                second_id.clone(),
                vec![HierarchyVisibilityTarget::PrimPath(
                    second_anchor.prim_path.clone(),
                )],
            ),
        ]));
    *app.world_mut().resource_mut::<CurrentHierarchyProjection>() =
        CurrentHierarchyProjection::from_shared_parts_with_visibility(
            std::sync::Arc::new(read_model.clone()),
            HierarchyPageIndex::from_read_model(&read_model),
            visibility_index,
        );

    let hide_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetHierarchyNodeVisibility {
            source: HierarchySource::BimClassification,
            node_id: group_id.clone(),
            visible: false,
        },
    );
    app.update();

    assert_eq!(
        *app.world().get::<Visibility>(first).unwrap(),
        Visibility::Hidden
    );
    assert_eq!(
        *app.world().get::<Visibility>(second).unwrap(),
        Visibility::Hidden
    );
    let events: Vec<_> =
        std::iter::from_fn(|| app.world_mut().resource_mut::<ViewportEventOutbox>().pop())
            .collect();
    assert!(events.iter().any(|event| {
        event.request_id.as_deref() == Some(hide_request.as_str())
            && matches!(
                event.event,
                ViewportEvent::HierarchyVisibilityChanged {
                    source: HierarchySource::BimClassification,
                    ref target,
                    visibility: HierarchyVisibilityState::Hidden,
                    ..
                } if target == &group_id
            )
    }));

    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetHierarchyNodeVisibility {
            source: HierarchySource::BimClassification,
            node_id: group_id,
            visible: true,
        },
    );
    app.update();
    assert_eq!(
        *app.world().get::<Visibility>(first).unwrap(),
        Visibility::Visible
    );
    assert_eq!(
        *app.world().get::<Visibility>(second).unwrap(),
        Visibility::Visible
    );
}
