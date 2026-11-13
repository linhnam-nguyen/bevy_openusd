#[cfg(test)]
mod tests {
    use bevy::prelude::*;
    use viewport_protocol::*;

    use crate::viewport::api::bridge::scene_query::dispatch_scene_query_commands;
    use crate::viewport::api::bridge::state::SceneSearchRequests;
    use crate::viewport::api::scene_query::SceneQueryService;
    use crate::viewport::api::{
        ActiveHierarchyProvider, CurrentHierarchyProjection, SceneAnchorIndex,
        ViewportCommandInbox, ViewportEventOutbox,
    };
    use crate::viewport::bim::test_fixtures;
    use crate::viewport::scene::SelectedTargets;
    use crate::viewport::semantic::SemanticSyncState;

    fn properties_test_app() -> App {
        let mut app = App::new();
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<CurrentHierarchyProjection>()
            .init_resource::<ActiveHierarchyProvider>()
            .init_resource::<SceneQueryService>()
            .init_resource::<SceneSearchRequests>()
            .init_resource::<SelectedTargets>()
            .insert_resource(SemanticSyncState::from_test_snapshot(
                test_fixtures::snapshot(),
            ))
            .add_systems(Update, dispatch_scene_query_commands);
        app
    }

    #[test]
    fn request_returns_authoritative_single_selection_rows() {
        let mut app = properties_test_app();
        let target = SceneAnchor::active_session("/World/WallA");
        let mut selection = SelectedTargets::default();
        selection
            .replace(SelectionReadModel {
                targets: vec![target.clone()],
                primary: Some(target.clone()),
            })
            .expect("selection is valid");
        let revision = selection.revision();
        app.world_mut().insert_resource(selection);

        let request_id = app
            .world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::RequestBimProperties);
        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("property read event");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        let ViewportEvent::BimPropertiesRead { properties, diff } = event.event else {
            panic!("expected BIM property read event");
        };
        assert!(diff.is_none());
        assert_eq!(properties.targets, vec![target]);
        assert_eq!(properties.selection_revision, revision);
        assert!(
            properties
                .groups
                .iter()
                .flat_map(|group| group.properties.iter())
                .any(|property| {
                    property.key == "Width"
                        && property.editable
                        && property.units.iter().any(|unit| {
                            unit.unit.as_str() == "mm"
                                && (unit.scale_to_canonical - 0.001).abs() < f64::EPSILON
                                && unit.offset_to_canonical.abs() < f64::EPSILON
                        })
                })
        );
    }
}
