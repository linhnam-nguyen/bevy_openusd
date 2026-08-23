#[cfg(test)]
mod tests {
    use bevy::prelude::*;
    use viewport_protocol::*;

    use crate::viewport::animation::UsdStageTime;
    use crate::viewport::api::bridge::ViewerSettingsState;
    use crate::viewport::api::bridge::commands::{
        apply_pending_renderer_cadence, apply_viewport_commands,
    };
    use crate::viewport::api::bridge::state::{EditorHistories, RuntimeMutationCoordinator};
    use crate::viewport::api::{
        SceneAnchorIndex, ViewportCommandInbox, ViewportEventOutbox, ViewportTreeCommandInbox,
    };
    use crate::viewport::app::cadence::RendererCadence;
    use crate::viewport::camera::CameraMount;
    use crate::viewport::physics::PhysicsActive;
    use crate::viewport::rendering::sampling::{
        DlssCameraActivation, DlssCapability, FsrVulkanCapability, SamplingCoordinatorState,
    };
    use crate::viewport::scene::visualization::DisplayToggles;
    use crate::viewport::scene::{SelectedPrim, SelectedTargets, SolariCapability};
    use crate::viewport::session::{LoaderTuning, ReloadRequest, Spawned, StageInfo};

    fn command_test_app() -> App {
        let mut app = App::new();
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .init_resource::<ViewportTreeCommandInbox>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<ReloadRequest>()
            .init_resource::<SelectedPrim>()
            .init_resource::<SelectedTargets>()
            .init_resource::<ViewerSettingsState>()
            .init_resource::<SamplingCoordinatorState>()
            .init_resource::<DlssCapability>()
            .init_resource::<FsrVulkanCapability>()
            .init_resource::<DlssCameraActivation>()
            .init_resource::<CameraMount>()
            .init_resource::<UsdStageTime>()
            .init_resource::<DisplayToggles>()
            .init_resource::<LoaderTuning>()
            .init_resource::<PhysicsActive>()
            .init_resource::<EditorHistories>()
            .init_resource::<RuntimeMutationCoordinator>()
            .init_resource::<Spawned>()
            .insert_resource(StageInfo {
                path: "fixtures/spinner.usda".to_owned(),
                ..default()
            })
            .add_systems(Update, apply_viewport_commands);
        app
    }

    #[test]
    fn renderer_configuration_applies_and_preserves_command_correlation() {
        let mut app = command_test_app();
        let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::SetRendererConfiguration {
                configuration: RendererConfiguration::default(),
            },
        );

        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("renderer configuration must publish a response");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        let ViewportEvent::PresentationChanged { presentation } = event.event else {
            panic!("renderer configuration should publish an authoritative presentation event");
        };
        assert_eq!(presentation.renderer, RendererConfiguration::default());
    }

    #[test]
    fn uniform_color_renderer_configuration_is_accepted_for_b2() {
        let mut app = command_test_app();
        let configuration = RendererConfiguration {
            render_mode: RenderMode::UniformColor,
            ..Default::default()
        };
        let request_id = app
            .world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SetRendererConfiguration { configuration });

        app.update();

        assert_eq!(
            app.world().resource::<DisplayToggles>().renderer,
            configuration
        );
        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("uniform renderer configuration must publish a response");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        let ViewportEvent::PresentationChanged { presentation } = event.event else {
            panic!("uniform renderer configuration should publish a presentation event");
        };
        assert_eq!(presentation.renderer.render_mode, RenderMode::UniformColor);
    }

    #[test]
    fn ray_traced_renderer_configuration_rejects_without_supported_solari() {
        let mut app = command_test_app();
        app.init_resource::<SolariCapability>();
        let before = app.world().resource::<DisplayToggles>().renderer;
        let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::SetRendererConfiguration {
                configuration: RendererConfiguration {
                    render_mode: RenderMode::RayTraced,
                    ..Default::default()
                },
            },
        );

        app.update();

        assert_eq!(app.world().resource::<DisplayToggles>().renderer, before);
        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("unsupported ray traced configuration must publish a rejection");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        assert!(matches!(
            event.event,
            ViewportEvent::CommandRejected { reason, .. }
                if reason.contains("unsupported")
        ));
    }

    #[test]
    fn renderer_fps_event_is_published_only_after_cadence_application() {
        let mut app = command_test_app();
        app.insert_resource(RendererCadence::new(Some(60)))
            .add_systems(
                Update,
                apply_pending_renderer_cadence.after(apply_viewport_commands),
            );
        let configuration = RendererConfiguration {
            preferred_fps: Some(120),
            ..Default::default()
        };
        let request_id = app
            .world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SetRendererConfiguration { configuration });

        app.update();

        let cadence = app.world().resource::<RendererCadence>();
        assert_eq!(cadence.effective_renderer_target_fps(), Some(120));
        assert_eq!(
            app.world().resource::<DisplayToggles>().renderer,
            configuration
        );
        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("applied FPS must publish a presentation event");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        let ViewportEvent::PresentationChanged { presentation } = event.event else {
            panic!("FPS application must publish a presentation event");
        };
        assert_eq!(presentation.renderer.preferred_fps, Some(120));
    }

    #[test]
    fn renderer_configuration_keeps_edges_independent_from_render_mode() {
        let mut app = command_test_app();
        let shaded_edges = RendererConfiguration {
            edges: true,
            render_mode: RenderMode::Shaded,
            ..Default::default()
        };
        let first_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::SetRendererConfiguration {
                configuration: shaded_edges,
            },
        );
        app.update();

        let first_event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("shaded edge configuration should publish a response");
        assert_eq!(
            first_event.request_id.as_deref(),
            Some(first_request.as_str())
        );
        assert_eq!(
            app.world().resource::<DisplayToggles>().renderer,
            shaded_edges
        );

        let wireframe_edges = RendererConfiguration {
            render_mode: RenderMode::Wireframe,
            ..shaded_edges
        };
        let second_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::SetRendererConfiguration {
                configuration: wireframe_edges,
            },
        );
        app.update();

        let second_event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("wireframe configuration should publish a response");
        assert_eq!(
            second_event.request_id.as_deref(),
            Some(second_request.as_str())
        );
        assert_eq!(
            app.world().resource::<DisplayToggles>().renderer,
            wireframe_edges
        );
        assert!(wireframe_edges.edges);
    }

    #[test]
    fn repeated_renderer_configuration_is_idempotent() {
        let mut app = command_test_app();
        let configuration = RendererConfiguration {
            grid: false,
            shadows: false,
            edges: true,
            render_mode: RenderMode::Wireframe,
            preferred_fps: Some(120),
        };

        let first_request = app
            .world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SetRendererConfiguration { configuration });
        app.update();
        let first_event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("first renderer configuration should publish a response");
        assert_eq!(
            first_event.request_id.as_deref(),
            Some(first_request.as_str())
        );
        assert_eq!(
            app.world().resource::<DisplayToggles>().renderer,
            configuration
        );

        let second_request = app
            .world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SetRendererConfiguration { configuration });
        app.update();
        let second_event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("repeated renderer configuration should publish a response");
        assert_eq!(
            second_event.request_id.as_deref(),
            Some(second_request.as_str())
        );
        assert_eq!(
            app.world().resource::<DisplayToggles>().renderer,
            configuration
        );
    }

    #[test]
    fn renderer_command_reaches_the_real_webrtc_gateway_boundary() {
        let mut app = command_test_app();
        let interface = crate::viewport::api::RenderServerInterface::default();
        app.insert_resource(interface.clone());
        app.add_systems(
            PreUpdate,
            crate::viewport::transport::webrtc::drain_remote_commands
                .before(apply_viewport_commands),
        );
        app.add_systems(
            PostUpdate,
            crate::viewport::transport::webrtc::publish_authoritative_events
                .after(apply_viewport_commands),
        );

        interface
            .submit_viewport_command(ViewportCommandEnvelope::new(
                "test-renderer-1",
                ViewportCommand::SetRendererConfiguration {
                    configuration: RendererConfiguration {
                        grid: false,
                        ..Default::default()
                    },
                },
            ))
            .expect("must submit");

        app.update();

        assert!(!app.world().resource::<DisplayToggles>().renderer.grid);
        let event = interface
            .pop_viewport_event()
            .expect("server must publish authoritative renderer state");
        assert_eq!(event.request_id.as_deref(), Some("test-renderer-1"));
        assert!(matches!(
            event.event,
            ViewportEvent::PresentationChanged { presentation } if !presentation.renderer.grid
        ));
    }

    #[test]
    fn pending_stream_fps_survives_a_same_frame_renderer_command() {
        let mut app = command_test_app();
        app.insert_resource(RendererCadence::new(Some(60)))
            .add_systems(
                Update,
                apply_pending_renderer_cadence.after(apply_viewport_commands),
            );
        app.world_mut()
            .resource_mut::<RendererCadence>()
            .request_stream(Some(120), 2);
        app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::SetRendererConfiguration {
                configuration: RendererConfiguration::default(),
            },
        );

        app.update();

        assert_eq!(
            app.world()
                .resource::<RendererCadence>()
                .effective_renderer_target_fps(),
            Some(120)
        );
    }
}
