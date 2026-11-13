use bevy::ecs::change_detection::DetectChanges;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::prelude::{Res, ResMut, Resource, Update};
use viewport_protocol::*;

use super::super::ViewerSettingsState;
use super::support::command_test_app;
use crate::viewport::api::bridge::commands::apply_viewport_commands;
use crate::viewport::api::{ViewportCommandInbox, ViewportEventOutbox};
use crate::viewport::rendering::sampling::{
    ActiveUpscaler, DlssCameraActivation, DlssCapability, SamplingCoordinatorState,
};
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
fn unsupported_sampling_command_rejects_without_mutating_applied_state() {
    let mut app = command_test_app();
    let before_settings = app.world().resource::<ViewerSettingsState>().0.clone();
    let before_renderer = app.world().resource::<DisplayToggles>().renderer;
    let request_ids = {
        let mut inbox = app.world_mut().resource_mut::<ViewportCommandInbox>();
        vec![inbox.send(ViewportCommand::SetSamplingPreference {
            preference: SamplingPreference { enabled: true },
        })]
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

#[test]
fn section_box_command_updates_authoritative_settings() {
    let mut app = command_test_app();
    let request_id = app
        .world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::SetSectionBox { enabled: true });

    app.update();

    assert!(
        app.world()
            .resource::<ViewerSettingsState>()
            .section_box_enabled()
    );
    let event = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("section-box command must publish an applied event");
    assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
    let ViewportEvent::ViewerSettingsChanged { settings } = event.event else {
        panic!("section-box command must publish viewer settings");
    };
    assert!(settings.section_box.enabled);

    let snapshot_request = app
        .world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::RequestSnapshot);
    app.update();
    let snapshot = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("reconnect snapshot must be available");
    assert_eq!(
        snapshot.request_id.as_deref(),
        Some(snapshot_request.as_str())
    );
    let ViewportEvent::Snapshot { state } = snapshot.event else {
        panic!("reconnect must receive a snapshot");
    };
    assert!(state.viewer_settings.section_box.enabled);
}

#[test]
fn selection_boundary_command_applies_renderer_owned_settings() {
    let mut app = command_test_app();
    let settings = SelectionPresentationSettings {
        boundary_enabled: false,
        boundary_color: ColorRgb8::new(0x10, 0x20, 0x30),
        ..SelectionPresentationSettings::default()
    };
    let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetSelectionPresentationSettings {
            settings: settings.clone(),
        },
    );

    app.update();

    assert_eq!(
        app.world().resource::<ViewerSettingsState>().0.selection,
        settings
    );
    let event = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("selection presentation must publish an applied event");
    assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
    let ViewportEvent::ViewerSettingsChanged { settings: applied } = event.event else {
        panic!("selection presentation must publish a viewer-settings event");
    };
    assert_eq!(applied.selection, settings);
}

#[test]
fn sampling_command_selects_dlss_and_publishes_authoritative_state() {
    let mut app = command_test_app();
    app.world_mut().resource_mut::<DlssCapability>().compiled = true;
    app.world_mut()
        .resource_mut::<DlssCapability>()
        .runtime_supported = true;
    let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetSamplingPreference {
            preference: SamplingPreference { enabled: true },
        },
    );

    app.update();

    let settings = &app.world().resource::<ViewerSettingsState>().0;
    assert!(settings.sampling.preference.enabled);
    assert_eq!(settings.sampling.provider, SamplingProvider::Dlss);
    let coordinator = app.world().resource::<SamplingCoordinatorState>();
    assert_eq!(coordinator.active, ActiveUpscaler::Dlss);
    assert!(coordinator.preference_enabled);
    assert!(app.world().resource::<DlssCameraActivation>().enabled);
    let event = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("sampling selection must publish an authoritative event");
    assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
    let ViewportEvent::ViewerSettingsChanged { settings } = event.event else {
        panic!("sampling selection must publish a viewer-settings event");
    };
    assert_eq!(settings.sampling.provider, SamplingProvider::Dlss);

    let snapshot_request = app
        .world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::RequestSnapshot);
    app.update();
    let snapshot = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("reconnect snapshot must be available after sampling selection");
    assert_eq!(
        snapshot.request_id.as_deref(),
        Some(snapshot_request.as_str())
    );
    let ViewportEvent::Snapshot { state } = snapshot.event else {
        panic!("sampling selection must be present in the reconnect snapshot");
    };
    assert_eq!(
        state.viewer_settings.sampling.provider,
        SamplingProvider::Dlss
    );
}

#[test]
fn sampling_command_rejects_when_dlss_is_unavailable_and_fsr_is_pending() {
    let mut app = command_test_app();
    let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetSamplingPreference {
            preference: SamplingPreference { enabled: true },
        },
    );

    app.update();

    assert_eq!(
        app.world()
            .resource::<ViewerSettingsState>()
            .0
            .sampling
            .provider,
        SamplingProvider::None
    );
    assert_eq!(
        app.world().resource::<SamplingCoordinatorState>().active,
        ActiveUpscaler::None
    );
    assert!(!app.world().resource::<DlssCameraActivation>().enabled);
    let event = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("unsupported sampling must publish a rejection");
    assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
    assert!(matches!(event.event, ViewportEvent::CommandRejected { .. }));
}

#[test]
fn sampling_command_disables_the_current_provider() {
    let mut app = command_test_app();
    app.world_mut()
        .insert_resource(DlssCapability::from_probe(true, true));
    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetSamplingPreference {
            preference: SamplingPreference { enabled: true },
        },
    );
    app.update();
    app.world_mut().resource_mut::<ViewportEventOutbox>().pop();

    let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetSamplingPreference {
            preference: SamplingPreference { enabled: false },
        },
    );
    app.update();

    let settings = &app.world().resource::<ViewerSettingsState>().0;
    assert!(!settings.sampling.preference.enabled);
    assert_eq!(settings.sampling.provider, SamplingProvider::None);
    let coordinator = app.world().resource::<SamplingCoordinatorState>();
    assert_eq!(coordinator.active, ActiveUpscaler::None);
    assert!(!coordinator.preference_enabled);
    assert!(!app.world().resource::<DlssCameraActivation>().enabled);
    let event = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("disabling sampling must publish an authoritative event");
    assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
    assert!(matches!(
        event.event,
        ViewportEvent::ViewerSettingsChanged { .. }
    ));
}

#[test]
fn unsupported_sampling_request_preserves_last_valid_state() {
    let mut app = command_test_app();
    app.world_mut()
        .insert_resource(DlssCapability::from_probe(true, true));
    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetSamplingPreference {
            preference: SamplingPreference { enabled: true },
        },
    );
    app.update();
    app.world_mut().resource_mut::<ViewportEventOutbox>().pop();

    let before_settings = app.world().resource::<ViewerSettingsState>().0.clone();
    let before_coordinator = *app.world().resource::<SamplingCoordinatorState>();
    let before_activation = *app.world().resource::<DlssCameraActivation>();
    app.world_mut()
        .insert_resource(DlssCapability::from_probe(true, false));
    let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetSamplingPreference {
            preference: SamplingPreference { enabled: true },
        },
    );

    app.update();

    assert_eq!(
        app.world().resource::<ViewerSettingsState>().0,
        before_settings
    );
    assert_eq!(
        *app.world().resource::<SamplingCoordinatorState>(),
        before_coordinator
    );
    assert_eq!(
        *app.world().resource::<DlssCameraActivation>(),
        before_activation
    );
    let event = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("unsupported sampling must publish a rejection");
    assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
    assert!(matches!(event.event, ViewportEvent::CommandRejected { .. }));
}
