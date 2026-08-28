#[cfg(test)]
mod tests {
    use openusd::usd::Stage;
    use viewport_protocol::*;

    use crate::viewport::api::{ViewportCommandInbox, ViewportEventOutbox};
    use crate::viewport::session::StageHandle;

    use super::super::support::command_test_app;

    fn stage_with_saved_prim() -> Stage {
        let stage = Stage::builder()
            .in_memory("bim_save_stage_test.usda")
            .expect("stage opens");
        stage
            .define_prim("/World/Saved")
            .expect("prim defines")
            .set_type_name("Xform")
            .expect("prim type authors");
        stage
    }

    #[test]
    fn save_stage_uses_current_stage_path_and_round_trips() {
        let temp_dir = tempfile::tempdir().expect("temporary directory creates");
        let path = temp_dir.path().join("saved.usda");
        let mut app = command_test_app();
        app.world_mut()
            .insert_non_send(usd_bevy::LiveStage::new(stage_with_saved_prim()));
        app.world_mut().insert_resource(StageHandle {
            path: path.clone(),
            error: None,
        });
        let request_id = app
            .world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SaveStage);

        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("save publishes one event");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        assert!(matches!(
            event.event,
            ViewportEvent::EditorCommandCompleted {
                operation: EditorOperation::SaveStage,
                changed_paths,
                ..
            } if changed_paths.is_empty()
        ));

        let reopened = Stage::open(path.to_str().expect("temporary path is valid UTF-8"))
            .expect("saved stage reopens");
        assert!(
            reopened
                .prim(openusd::sdf::path("/World/Saved").expect("prim path parses"))
                .type_name()
                .expect("prim type reads")
                .is_some(),
            "saved stage retains authored prim"
        );
    }
}
