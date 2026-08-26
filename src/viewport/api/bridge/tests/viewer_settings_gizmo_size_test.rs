use viewport_protocol::{SelectionPresentationSettings, ViewportCommand, ViewportEvent};

use super::super::ViewerSettingsState;
use super::support::command_test_app;
use crate::viewport::api::{ViewportCommandInbox, ViewportEventOutbox};

#[test]
fn selection_gizmo_size_command_updates_authoritative_settings() {
    let mut app = command_test_app();
    let mut settings = SelectionPresentationSettings::default();
    settings.gizmo_size_level = 7;
    let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetSelectionPresentationSettings {
            settings: settings.clone(),
        },
    );

    app.update();

    assert_eq!(
        app.world()
            .resource::<ViewerSettingsState>()
            .selection()
            .gizmo_size_level,
        7
    );
    let event = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("selection gizmo size must publish an applied event");
    assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
    let ViewportEvent::ViewerSettingsChanged { settings: applied } = event.event else {
        panic!("selection gizmo size must publish a viewer-settings event");
    };
    assert_eq!(applied.selection.gizmo_size_level, 7);
}
