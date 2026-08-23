use bevy::ecs::change_detection::DetectChanges;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::prelude::{Res, ResMut, Resource, Update};
use viewport_protocol::*;

use super::super::ViewerSettingsState;
use super::support::command_test_app;
use crate::viewport::api::bridge::commands::apply_viewport_commands;
use crate::viewport::api::{ViewportCommandInbox, ViewportEventOutbox};
use crate::viewport::scene::visualization::DisplayToggles;

#[derive(Resource, Default)]
struct ViewerSettingsChangeCount(u32);

fn count_viewer_settings_changes(
    mut count: ResMut<ViewerSettingsChangeCount>,
    settings: Res<ViewerSettingsState>,
) {
    if settings.is_changed() {
        count.0 += 1;
    }
}

#[test]
fn environment_command_applies_all_environment_fields_for_b2_2() {
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
        app.world().resource::<ViewerSettingsState>().0,
        ViewerSettingsReadModel {
            environment: environment.clone(),
            ..ViewerSettingsReadModel::default()
        }
    );
    let presentation = &app.world().resource::<DisplayToggles>().renderer;
    assert_eq!(presentation.render_mode, RenderMode::Shaded);
    let event = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("grid environment command must publish an applied event");
    assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
    let ViewportEvent::ViewerSettingsChanged { settings } = event.event else {
        panic!("grid environment settings must publish an applied event");
    };
    assert_eq!(settings.environment.grid_color, environment.grid_color);
    assert_eq!(settings.environment, environment);
}

#[test]
fn identical_environment_command_does_not_mark_settings_changed() {
    let mut app = command_test_app();
    app.init_resource::<ViewerSettingsChangeCount>()
        .add_systems(
            Update,
            count_viewer_settings_changes.after(apply_viewport_commands),
        );
    app.update();
    app.world_mut()
        .resource_mut::<ViewerSettingsChangeCount>()
        .0 = 0;

    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetEnvironmentSettings {
            settings: ViewerEnvironmentSettings::default(),
        },
    );
    app.update();

    assert_eq!(app.world().resource::<ViewerSettingsChangeCount>().0, 0);
    assert!(
        app.world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .is_some()
    );
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
