#[cfg(test)]
mod tests {
    use bevy::prelude::*;
    use openusd::sdf::Value;
    use viewport_protocol::*;

    use crate::viewport::api::{ViewportCommandInbox, ViewportEventOutbox};
    use crate::viewport::scene::SelectedTargets;

    use super::super::support::command_test_app;

    fn stage_with_widths() -> openusd::usd::Stage {
        let stage = openusd::usd::Stage::builder()
            .in_memory("bim_atomic_edit_test.usda")
            .expect("stage opens");
        stage
            .define_prim("/World")
            .expect("world defines")
            .set_type_name("Xform")
            .expect("world type authors");
        for (path, width) in [("/World/A", 1.0), ("/World/B", 2.0)] {
            stage
                .define_prim(path)
                .expect("target defines")
                .set_type_name("Xform")
                .expect("target type authors");
            stage
                .prim(openusd::sdf::path(path).expect("path parses"))
                .create_attribute("Width", "double")
                .expect("attribute creates")
                .set_custom(true)
                .expect("custom flag authors")
                .set(Value::Double(width))
                .expect("attribute value authors");
        }
        stage
    }

    fn mutation(path: &str, expected: f64, next: f64) -> BimPropertyMutation {
        BimPropertyMutation {
            target: SceneAnchor::active_session(path),
            property: "Width".to_owned(),
            value: serde_json::json!(next),
            input_unit: None,
            expected_old_value: usd_model::CanonicalValue::Real(expected),
        }
    }

    fn read_width(app: &App, path: &str) -> f64 {
        let live = app
            .world()
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("live stage");
        match live
            .stage
            .prim(openusd::sdf::path(path).expect("path parses"))
            .attribute("Width")
            .get::<Value>()
            .expect("width reads")
        {
            Some(Value::Double(value)) => value,
            other => panic!("expected double Width, got {other:?}"),
        }
    }

    fn select_targets(app: &mut App) -> u64 {
        let mut selection = SelectedTargets::default();
        selection
            .replace(SelectionReadModel {
                targets: vec![
                    SceneAnchor::active_session("/World/A"),
                    SceneAnchor::active_session("/World/B"),
                ],
                primary: Some(SceneAnchor::active_session("/World/A")),
            })
            .expect("selection is valid");
        let revision = selection.revision();
        app.world_mut().insert_resource(selection);
        revision
    }

    #[test]
    fn multi_selection_edit_is_atomic_and_has_one_undo_step() {
        let mut app = command_test_app();
        app.world_mut()
            .insert_non_send(usd_bevy::LiveStage::new(stage_with_widths()));
        let selection_revision = select_targets(&mut app);
        let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::EditBimProperties {
                selection_revision,
                mutations: vec![
                    mutation("/World/A", 1.0, 10.0),
                    mutation("/World/B", 2.0, 20.0),
                ],
            },
        );

        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("batch edit publishes one event");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        assert!(matches!(
            event.event,
            ViewportEvent::BimPropertyBatchEditCompleted {
                applied: true,
                outcomes,
                state,
                ..
            } if outcomes.len() == 2 && outcomes.iter().all(|outcome| outcome.status == BimPropertyEditStatus::Applied) && state.can_undo
        ));
        assert_eq!(read_width(&app, "/World/A"), 10.0);
        assert_eq!(read_width(&app, "/World/B"), 20.0);

        let undo_request = app
            .world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::UndoEditor);
        app.update();
        let undo_event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("undo publishes one event");
        assert_eq!(
            undo_event.request_id.as_deref(),
            Some(undo_request.as_str())
        );
        assert!(matches!(
            undo_event.event,
            ViewportEvent::EditorCommandCompleted {
                operation: EditorOperation::Undo,
                ..
            }
        ));
        assert_eq!(read_width(&app, "/World/A"), 1.0);
        assert_eq!(read_width(&app, "/World/B"), 2.0);
    }

    #[test]
    fn stale_member_rejects_the_whole_batch_without_stage_changes() {
        let mut app = command_test_app();
        app.world_mut()
            .insert_non_send(usd_bevy::LiveStage::new(stage_with_widths()));
        let selection_revision = select_targets(&mut app);
        app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::EditBimProperties {
                selection_revision,
                mutations: vec![
                    mutation("/World/A", 1.0, 10.0),
                    mutation("/World/B", 99.0, 20.0),
                ],
            },
        );

        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("rejected batch publishes one event");
        assert!(matches!(
            event.event,
            ViewportEvent::BimPropertyBatchEditCompleted {
                applied: false,
                outcomes,
                ..
            } if outcomes.len() == 2 && outcomes.iter().all(|outcome| outcome.status == BimPropertyEditStatus::Rejected)
        ));
        assert_eq!(read_width(&app, "/World/A"), 1.0);
        assert_eq!(read_width(&app, "/World/B"), 2.0);
    }

    #[test]
    fn stale_selection_revision_rejects_the_whole_batch_without_stage_changes() {
        let mut app = command_test_app();
        app.world_mut()
            .insert_non_send(usd_bevy::LiveStage::new(stage_with_widths()));
        let selection_revision = select_targets(&mut app);
        app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::EditBimProperties {
                selection_revision: selection_revision.saturating_sub(1),
                mutations: vec![
                    mutation("/World/A", 1.0, 10.0),
                    mutation("/World/B", 2.0, 20.0),
                ],
            },
        );

        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("stale selection publishes one event");
        assert!(matches!(
            event.event,
            ViewportEvent::BimPropertyBatchEditCompleted {
                applied: false,
                outcomes,
                ..
            } if outcomes.iter().all(|outcome| outcome.status == BimPropertyEditStatus::Rejected)
        ));
        assert_eq!(read_width(&app, "/World/A"), 1.0);
        assert_eq!(read_width(&app, "/World/B"), 2.0);
    }

    #[test]
    fn batch_targets_must_match_the_authoritative_selection() {
        let mut app = command_test_app();
        app.world_mut()
            .insert_non_send(usd_bevy::LiveStage::new(stage_with_widths()));
        let selection_revision = select_targets(&mut app);
        app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::EditBimProperties {
                selection_revision,
                mutations: vec![mutation("/World/A", 1.0, 10.0)],
            },
        );

        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("target mismatch publishes one event");
        assert!(matches!(
            event.event,
            ViewportEvent::BimPropertyBatchEditCompleted {
                applied: false,
                outcomes,
                ..
            } if outcomes.len() == 1
                && outcomes[0].status == BimPropertyEditStatus::Rejected
        ));
        assert_eq!(read_width(&app, "/World/A"), 1.0);
        assert_eq!(read_width(&app, "/World/B"), 2.0);
    }
}
