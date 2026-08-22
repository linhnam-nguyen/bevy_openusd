use viewport_protocol::*;

use super::super::ViewerSettingsState;
use super::support::command_test_app;
use crate::viewport::api::{ViewportCommandInbox, ViewportEventOutbox};
use crate::viewport::scene::visualization::DisplayToggles;

#[test]
fn environment_command_updates_only_supplementary_settings() {
    let mut app = command_test_app();
    let environment = ViewerEnvironmentSettings {
        background_color: ColorRgb8::new(1, 2, 3),
        ..ViewerEnvironmentSettings::default()
    };
    let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetEnvironmentSettings {
            settings: environment.clone(),
        },
    );

    app.update();

    assert_eq!(
        app.world().resource::<ViewerSettingsState>().0.environment,
        environment
    );
    let presentation = &app.world().resource::<DisplayToggles>().renderer;
    assert_eq!(presentation.render_mode, RenderMode::Shaded);
    let event = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("settings command must publish an authoritative event");
    assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
    let ViewportEvent::ViewerSettingsChanged { settings } = event.event else {
        panic!("settings command should publish a settings event");
    };
    assert_eq!(settings.environment, environment);
}

#[test]
fn unsupported_settings_commands_reject_without_mutating_applied_state() {
    let mut app = command_test_app();
    let before_settings = app.world().resource::<ViewerSettingsState>().0.clone();
    let before_renderer = app.world().resource::<DisplayToggles>().renderer;
    let request_ids = {
        let mut inbox = app.world_mut().resource_mut::<ViewportCommandInbox>();
        vec![
            inbox.send(ViewportCommand::SetSamplingPreference {
                preference: SamplingPreference { enabled: true },
            }),
            inbox.send(ViewportCommand::SetSelectionPresentationSettings {
                settings: SelectionPresentationSettings::default(),
            }),
            inbox.send(ViewportCommand::SetSectionBox { enabled: true }),
        ]
    };

    app.update();

    assert_eq!(
        app.world().resource::<ViewerSettingsState>().0,
        before_settings
    );
    assert_eq!(
        app.world().resource::<DisplayToggles>().renderer,
        before_renderer
    );
    let events: Vec<_> =
        std::iter::from_fn(|| app.world_mut().resource_mut::<ViewportEventOutbox>().pop())
            .collect();
    assert_eq!(events.len(), request_ids.len());
    for (event, request_id) in events.iter().zip(request_ids) {
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        assert!(matches!(event.event, ViewportEvent::CommandRejected { .. }));
    }
}
