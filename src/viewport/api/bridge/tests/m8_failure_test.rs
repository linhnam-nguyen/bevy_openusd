#[cfg(test)]
mod tests {
    use bevy::prelude::*;
    use openusd::sdf::Value;
    use openusd::usd::Stage;
    use viewport_protocol::*;

    use crate::viewport::api::bridge::state::{EditorHistories, EditorHistoryDomain};
    use crate::viewport::api::{ViewportCommandInbox, ViewportEventOutbox};
    use crate::viewport::bim::test_fixtures;
    use crate::viewport::semantic::SemanticSyncState;

    use super::super::support::command_test_app;

    fn stage_with_measured_width() -> Stage {
        let stage = Stage::builder()
            .in_memory("m8_invalid_unit_test.usda")
            .expect("stage opens");
        stage
            .define_prim("/World")
            .expect("world defines")
            .set_type_name("Xform")
            .expect("world type authors");
        stage
            .define_prim("/World/WallA")
            .expect("wall defines")
            .set_type_name("Xform")
            .expect("wall type authors");
        stage
            .prim(openusd::sdf::path("/World/WallA").expect("path parses"))
            .create_attribute("Width", "double")
            .expect("attribute creates")
            .set_custom(true)
            .expect("custom flag authors")
            .set(Value::Double(200.0))
            .expect("attribute value authors");
        stage
    }

    fn read_width(app: &App) -> f64 {
        let live = app
            .world()
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("live stage");
        match live
            .stage
            .prim(openusd::sdf::path("/World/WallA").expect("path parses"))
            .attribute("Width")
            .get::<Value>()
            .expect("width reads")
        {
            Some(Value::Double(value)) => value,
            other => panic!("expected double Width, got {other:?}"),
        }
    }

    #[test]
    fn invalid_unit_edit_is_rejected_without_mutating_live_stage() {
        let mut app = command_test_app();
        app.world_mut()
            .insert_non_send(usd_bevy::LiveStage::new(stage_with_measured_width()));
        app.world_mut()
            .insert_resource(SemanticSyncState::from_test_snapshot(
                test_fixtures::snapshot(),
            ));
        let before_revision = app
            .world()
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("live stage")
            .current_revision()
            .0;
        let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::EditBimProperty {
                mutation: BimPropertyMutation {
                    target: SceneAnchor::active_session("/World/WallA"),
                    property: "Width".to_owned(),
                    value: serde_json::json!(250.0),
                    input_unit: Some(usd_model::UnitId::new("invalid-unit")),
                    expected_old_value: usd_model::CanonicalValue::Real(0.2),
                },
            },
        );

        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("invalid unit publishes one authoritative outcome");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        assert!(matches!(
            event.event,
            ViewportEvent::BimPropertyEditCompleted { outcome, .. }
                if outcome.status == BimPropertyEditStatus::Rejected
                    && outcome.reason.as_deref().is_some_and(|reason| reason.contains("unknown unit invalid-unit"))
        ));
        assert_eq!(read_width(&app), 200.0);
        assert_eq!(
            app.world()
                .get_non_send::<usd_bevy::LiveStage>()
                .expect("live stage")
                .current_revision()
                .0,
            before_revision
        );
        assert!(!app.world().resource::<EditorHistories>().state().is_dirty);
    }

    #[test]
    fn save_rejection_preserves_dirty_state_and_has_no_completion() {
        let mut app = command_test_app();
        app.world_mut()
            .insert_non_send(usd_bevy::LiveStage::new(stage_with_measured_width()));
        app.world_mut()
            .resource_mut::<EditorHistories>()
            .record(EditorHistoryDomain::Authoring);
        let request_id = app
            .world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SaveStage);

        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("save rejection publishes one authoritative event");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        assert!(matches!(
            event.event,
            ViewportEvent::CommandRejected { reason, .. }
                if reason.contains("current stage has no local save path")
        ));
        let state = app.world().resource::<EditorHistories>().state();
        assert!(state.can_undo);
        assert!(state.is_dirty);
        assert!(!state.can_redo);
    }
}
