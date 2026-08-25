use bevy::prelude::*;
use viewport_protocol::*;

use super::super::ViewerSettingsState;
use super::support::command_test_app;
use crate::viewport::api::{ViewportCommandInbox, ViewportEventOutbox};
use crate::viewport::rendering::sampling::{DlssCameraActivation, DlssCapability};
use crate::viewport::scene::SolariCapability;
use crate::viewport::scene::visualization::DisplayToggles;

#[test]
fn supported_viewer_settings_round_trip_through_bridge_and_snapshot() {
    let mut app = command_test_app();
    let renderer = RendererConfiguration {
        grid: false,
        shadows: false,
        edges: true,
        render_mode: RenderMode::Wireframe,
        preferred_fps: Some(120),
    };
    let environment = ViewerEnvironmentSettings {
        grid_color: ColorRgb8::new(0x10, 0x20, 0x30),
        background_color: ColorRgb8::new(0x40, 0x50, 0x60),
        default_surface_color: ColorRgb8::new(0x70, 0x80, 0x90),
    };
    let selection = SelectionPresentationSettings {
        boundary_enabled: false,
        boundary_color: ColorRgb8::new(0xA0, 0xB0, 0xC0),
        color_change_enabled: true,
        selection_color: ColorRgb8::new(0x11, 0x22, 0x33),
        hover_color_change_enabled: true,
        hover_color: ColorRgb8::new(0x44, 0x55, 0x66),
    };
    let commands = [
        ViewportCommand::SetRendererConfiguration {
            configuration: renderer,
        },
        ViewportCommand::SetGroundGridOrigin {
            origin: GroundGridOrigin::WorldOrigin,
        },
        ViewportCommand::SetEnvironmentSettings {
            settings: environment.clone(),
        },
        ViewportCommand::SetSamplingPreference {
            preference: SamplingPreference { enabled: false },
        },
        ViewportCommand::SetSelectionPresentationSettings {
            settings: selection.clone(),
        },
        ViewportCommand::SetSectionBox { enabled: true },
    ];
    let request_ids: Vec<_> = commands
        .into_iter()
        .map(|command| {
            app.world_mut()
                .resource_mut::<ViewportCommandInbox>()
                .send(command)
        })
        .collect();

    app.update();

    let events: Vec<_> =
        std::iter::from_fn(|| app.world_mut().resource_mut::<ViewportEventOutbox>().pop())
            .collect();
    assert_eq!(events.len(), request_ids.len());
    for request_id in request_ids {
        assert!(
            events
                .iter()
                .any(|event| { event.request_id.as_deref() == Some(request_id.as_str()) })
        );
    }

    let presentation = &app.world().resource::<DisplayToggles>().renderer;
    assert_eq!(*presentation, renderer);
    assert_eq!(
        app.world().resource::<DisplayToggles>().ground_grid_origin,
        GroundGridOrigin::WorldOrigin
    );
    let settings = &app.world().resource::<ViewerSettingsState>().0;
    assert_eq!(settings.environment, environment);
    assert_eq!(settings.selection, selection);
    assert!(settings.section_box.enabled);
    assert!(!settings.sampling.preference.enabled);
    assert_eq!(settings.sampling.provider, SamplingProvider::None);

    let snapshot_request = app
        .world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::RequestSnapshot);
    app.update();
    let snapshot = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("the bridge must expose the applied settings through snapshot");
    assert_eq!(
        snapshot.request_id.as_deref(),
        Some(snapshot_request.as_str())
    );
    let ViewportEvent::Snapshot { state } = snapshot.event else {
        panic!("the reconnect response must be a snapshot");
    };
    assert_eq!(state.presentation.renderer, renderer);
    assert_eq!(
        state.presentation.ground_grid_origin,
        GroundGridOrigin::WorldOrigin
    );
    assert_eq!(state.viewer_settings.environment, environment);
    assert_eq!(state.viewer_settings.selection, selection);
    assert!(state.viewer_settings.section_box.enabled);
}

#[test]
fn invalid_renderer_configuration_is_rejected_before_state_mutation() {
    let mut app = command_test_app();
    let before = app.world().resource::<DisplayToggles>().renderer;
    let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetRendererConfiguration {
            configuration: RendererConfiguration {
                preferred_fps: Some(241),
                ..RendererConfiguration::default()
            },
        },
    );

    app.update();

    assert_eq!(app.world().resource::<DisplayToggles>().renderer, before);
    let event = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("invalid renderer settings must publish a rejection");
    assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
    assert!(matches!(event.event, ViewportEvent::CommandRejected { .. }));
}

#[test]
fn unsupported_capabilities_reject_without_mutating_authoritative_settings() {
    let mut app = command_test_app();
    app.init_resource::<SolariCapability>();
    let before_renderer = app.world().resource::<DisplayToggles>().renderer;
    let before_settings = app.world().resource::<ViewerSettingsState>().0.clone();
    let ray_traced_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetRendererConfiguration {
            configuration: RendererConfiguration {
                render_mode: RenderMode::RayTraced,
                ..RendererConfiguration::default()
            },
        },
    );
    let sampling_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetSamplingPreference {
            preference: SamplingPreference { enabled: true },
        },
    );

    app.update();

    assert_eq!(
        app.world().resource::<DisplayToggles>().renderer,
        before_renderer
    );
    assert_eq!(
        app.world().resource::<ViewerSettingsState>().0,
        before_settings
    );
    let events: Vec<_> =
        std::iter::from_fn(|| app.world_mut().resource_mut::<ViewportEventOutbox>().pop())
            .collect();
    assert_eq!(events.len(), 2);
    assert!(events.iter().any(|event| {
        event.request_id.as_deref() == Some(ray_traced_request.as_str())
            && matches!(event.event, ViewportEvent::CommandRejected { .. })
    }));
    assert!(events.iter().any(|event| {
        event.request_id.as_deref() == Some(sampling_request.as_str())
            && matches!(event.event, ViewportEvent::CommandRejected { .. })
    }));
}

#[test]
fn supported_capabilities_authorize_ray_traced_and_dlss_sampling() {
    let mut app = command_test_app();
    app.insert_resource(SolariCapability {
        compiled: true,
        device_supported: true,
        scene_eligible: true,
    });
    app.world_mut().resource_mut::<DlssCapability>().compiled = true;
    app.world_mut()
        .resource_mut::<DlssCapability>()
        .runtime_supported = true;

    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetRendererConfiguration {
            configuration: RendererConfiguration {
                render_mode: RenderMode::RayTraced,
                ..RendererConfiguration::default()
            },
        },
    );
    app.update();
    let ray_traced_event = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("supported Ray Traced must publish an applied event");
    assert!(matches!(
        ray_traced_event.event,
        ViewportEvent::PresentationChanged { presentation }
            if presentation.renderer.render_mode == RenderMode::RayTraced
    ));

    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetSamplingPreference {
            preference: SamplingPreference { enabled: true },
        },
    );
    app.update();
    let sampling_event = app
        .world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("supported DLSS must publish an applied event");
    assert!(matches!(
        sampling_event.event,
        ViewportEvent::ViewerSettingsChanged { settings }
            if settings.sampling.preference.enabled
                && settings.sampling.provider == SamplingProvider::Dlss
    ));
    assert!(app.world().resource::<DlssCameraActivation>().enabled);
}
