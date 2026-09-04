#[cfg(test)]
mod tests {
    use bevy::prelude::*;
    use viewport_protocol::{SelectionReadModel, ViewportCommand, ViewportEvent};

    use crate::viewport::api::bridge::bim_properties_lifecycle::{
        PendingBimProperties, replay_pending_bim_properties,
    };
    use crate::viewport::api::bridge::scene_query::dispatch_scene_query_commands;
    use crate::viewport::api::bridge::state::SceneSearchRequests;
    use crate::viewport::api::bridge::tests::support;
    use crate::viewport::api::scene_query::SceneQueryService;
    use crate::viewport::api::{
        CurrentHierarchyProjection, SceneAnchorIndex, ViewportCommandInbox, ViewportEventOutbox,
    };
    use crate::viewport::bim::test_fixtures;
    use crate::viewport::scene::SelectedTargets;
    use crate::viewport::semantic::SemanticSyncState;
    use crate::viewport::session::StageInfo;

    #[test]
    fn pre_readiness_property_request_replays_for_the_matching_generation() {
        let target = viewport_protocol::SceneAnchor::active_session("/World/WallA");
        let mut selection = SelectedTargets::default();
        selection
            .replace(SelectionReadModel {
                targets: vec![target.clone()],
                primary: Some(target.clone()),
            })
            .expect("selection is valid");
        let mut app = App::new();
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<CurrentHierarchyProjection>()
            .init_resource::<SceneSearchRequests>()
            .init_resource::<SceneQueryService>()
            .insert_resource(selection)
            .insert_resource(SemanticSyncState::default())
            .insert_resource(StageInfo {
                activation_generation: 7,
                ..Default::default()
            })
            .init_resource::<PendingBimProperties>()
            .add_systems(
                Update,
                (dispatch_scene_query_commands, replay_pending_bim_properties).chain(),
            );

        let request_id = app
            .world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::RequestBimProperties);
        app.update();
        assert!(app.world().resource::<PendingBimProperties>().has_request());
        assert!(
            app.world_mut()
                .resource_mut::<ViewportEventOutbox>()
                .take_published()
                .is_empty()
        );

        app.world_mut()
            .insert_resource(SemanticSyncState::from_test_snapshot(
                test_fixtures::snapshot(),
            ));
        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("pre-readiness request eventually emits a result");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        assert!(matches!(
            event.event,
            ViewportEvent::BimPropertiesRead { .. }
        ));
        assert!(!app.world().resource::<PendingBimProperties>().has_request());
    }

    #[test]
    fn progressive_bim_request_replays_when_anchor_is_unprojected() {
        let target = viewport_protocol::SceneAnchor::active_session("/World/WallA");
        let mut app = support::command_test_app();
        app.init_resource::<SceneSearchRequests>()
            .init_resource::<SceneQueryService>()
            .insert_resource(SemanticSyncState::default())
            .insert_resource(StageInfo {
                activation_generation: 1,
                ..Default::default()
            })
            .init_resource::<PendingBimProperties>()
            .add_systems(
                Update,
                (dispatch_scene_query_commands, replay_pending_bim_properties).chain(),
            );

        // Select an unprojected target (no Bevy entity in SceneAnchorIndex yet)
        app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::SelectTarget {
                target: Some(target.clone()),
            },
        );
        app.update();

        // Selection is accepted in SelectedTargets even if SelectedPrim is None
        assert_eq!(
            app.world().resource::<SelectedTargets>().0.primary,
            Some(target)
        );
        assert!(
            app.world()
                .resource::<crate::viewport::scene::SelectedPrim>()
                .0
                .is_none()
        );

        // Request BIM properties before semantic snapshot arrives
        let request_id = app
            .world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::RequestBimProperties);
        app.update();
        assert!(app.world().resource::<PendingBimProperties>().has_request());

        // Snapshot arrives during progressive projection
        app.world_mut()
            .insert_resource(SemanticSyncState::from_test_snapshot(
                test_fixtures::snapshot(),
            ));
        app.update();

        // Pending BIM properties query resolves successfully for the unprojected anchor
        let mut outbox = app.world_mut().resource_mut::<ViewportEventOutbox>();
        let events = outbox.take_published();
        let bim_event = events
            .into_iter()
            .find(|e| e.request_id.as_deref() == Some(request_id.as_str()))
            .expect("bim properties event must be published");
        assert!(matches!(
            bim_event.event,
            ViewportEvent::BimPropertiesRead { .. }
        ));
        assert!(!app.world().resource::<PendingBimProperties>().has_request());
    }
}
