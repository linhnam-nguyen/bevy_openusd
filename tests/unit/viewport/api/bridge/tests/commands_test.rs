#[cfg(test)]
mod tests {
    use bevy::prelude::*;
    use viewport_protocol::*;

    use crate::viewport::animation::UsdStageTime;
    use crate::viewport::api::bridge::ViewerSettingsState;
    use crate::viewport::api::bridge::commands::apply_viewport_commands;
    use crate::viewport::api::bridge::state::{EditorHistories, RuntimeMutationCoordinator};
    use crate::viewport::api::{
        SceneAnchorIndex, ViewportCommandInbox, ViewportEventOutbox, ViewportTreeCommandInbox,
    };
    use crate::viewport::camera::{ArcballCamera, CameraMount, CameraOrientationState, FlyTo};
    use crate::viewport::physics::PhysicsActive;
    use crate::viewport::rendering::sampling::{
        DlssCameraActivation, DlssCapability, SamplingCoordinatorState,
    };
    use crate::viewport::scene::visualization::DisplayToggles;
    use crate::viewport::scene::{SelectedPrim, SelectedTargets};
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
            .init_resource::<DlssCameraActivation>()
            .init_resource::<CameraMount>()
            .init_resource::<CameraOrientationState>()
            .init_resource::<FlyTo>()
            .init_resource::<UsdStageTime>()
            .init_resource::<DisplayToggles>()
            .init_resource::<LoaderTuning>()
            .init_resource::<PhysicsActive>()
            .init_resource::<EditorHistories>()
            .init_resource::<RuntimeMutationCoordinator>()
            .init_resource::<crate::viewport::bim::BimClassificationFieldCatalogueState>()
            .init_resource::<Spawned>()
            .insert_resource(StageInfo {
                path: "fixtures/spinner.usda".to_owned(),
                ..default()
            })
            .add_systems(Update, apply_viewport_commands);
        app
    }

    #[test]
    fn commands_update_runtime_state_and_publish_correlated_events() {
        let mut app = command_test_app();
        let request_ids = {
            let mut inbox = app.world_mut().resource_mut::<ViewportCommandInbox>();
            vec![
                inbox.send(ViewportCommand::SetOverlay {
                    overlay: OverlayKind::Wireframe,
                    enabled: true,
                }),
                inbox.send(ViewportCommand::SetPlayback { playing: true }),
                inbox.send(ViewportCommand::Seek { seconds: 999.0 }),
                inbox.send(ViewportCommand::SetPhysicsRunning { running: true }),
            ]
        };

        app.update();

        assert_eq!(
            app.world()
                .resource::<DisplayToggles>()
                .renderer
                .render_mode,
            RenderMode::Wireframe
        );
        assert!(app.world().resource::<UsdStageTime>().playing);
        assert_eq!(app.world().resource::<UsdStageTime>().seconds, 1.0 / 24.0);
        assert!(app.world().resource::<PhysicsActive>().0);

        let events: Vec<_> =
            std::iter::from_fn(|| app.world_mut().resource_mut::<ViewportEventOutbox>().pop())
                .collect();
        assert_eq!(events.len(), 4);
        assert_eq!(
            events[0].request_id.as_deref(),
            Some(request_ids[0].as_str())
        );
        assert!(matches!(
            events[0].event,
            ViewportEvent::PresentationChanged { .. }
        ));
        assert_eq!(
            events[1].request_id.as_deref(),
            Some(request_ids[1].as_str())
        );
        assert!(matches!(
            events[1].event,
            ViewportEvent::TimelineChanged { .. }
        ));
        assert_eq!(
            events[2].request_id.as_deref(),
            Some(request_ids[2].as_str())
        );
        assert!(matches!(
            events[2].event,
            ViewportEvent::TimelineChanged { .. }
        ));
        assert_eq!(
            events[3].request_id.as_deref(),
            Some(request_ids[3].as_str())
        );
        assert!(matches!(
            events[3].event,
            ViewportEvent::PhysicsChanged { running: true }
        ));
    }

    #[test]
    fn standard_view_switches_mounted_camera_to_arcball_without_losing_rig_state() {
        let mut app = command_test_app();
        app.world_mut().spawn((
            Camera3d::default(),
            Transform::default(),
            ArcballCamera {
                focus: Vec3::new(1.0, 2.0, 3.0),
                distance: 7.5,
                zoom_target: 7.5,
                ..Default::default()
            },
        ));
        *app.world_mut().resource_mut::<CameraMount>() = CameraMount::Mounted {
            prim_path: "/World/Camera".to_owned(),
        };
        let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::SetStandardView {
                view: StandardView::Top,
            },
        );

        app.update();

        assert!(matches!(
            app.world().resource::<CameraMount>(),
            CameraMount::Arcball
        ));
        let fly_to = app.world().resource::<FlyTo>();
        assert_eq!(fly_to.target_focus, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(fly_to.target_distance, 7.5);
        assert_eq!(fly_to.target_elevation, Some(core::f32::consts::FRAC_PI_2));
        let events: Vec<_> =
            std::iter::from_fn(|| app.world_mut().resource_mut::<ViewportEventOutbox>().pop())
                .collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].request_id.as_deref(), Some(request_id.as_str()));
        assert!(matches!(
            events[0].event,
            ViewportEvent::CameraSourceChanged {
                source: CameraSource::Arcball
            }
        ));
        assert_eq!(events[1].request_id.as_deref(), Some(request_id.as_str()));
        assert!(matches!(
            events[1].event,
            ViewportEvent::CameraStandardViewStarted {
                view: StandardView::Top
            }
        ));
    }

    #[test]
    fn grid_origin_command_updates_presentation_state_and_event() {
        let mut app = command_test_app();
        let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::SetGroundGridOrigin {
                origin: GroundGridOrigin::WorldOrigin,
            },
        );

        app.update();

        assert_eq!(
            app.world().resource::<DisplayToggles>().ground_grid_origin,
            GroundGridOrigin::WorldOrigin
        );
        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("grid-origin command publishes a presentation event");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        let ViewportEvent::PresentationChanged { presentation } = event.event else {
            panic!("expected presentation change");
        };
        assert_eq!(
            presentation.ground_grid_origin,
            GroundGridOrigin::WorldOrigin
        );
    }

    #[test]
    fn snapshot_contains_only_logical_viewport_state() {
        let mut app = command_test_app();
        let request_id = app
            .world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::RequestSnapshot);

        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("snapshot command must emit a response");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        let ViewportEvent::Snapshot { state } = event.event else {
            panic!("expected a snapshot event");
        };
        assert_eq!(state.stage.display_name, "fixtures/spinner.usda");
        assert!(state.scene.prims.is_empty());
        assert!(state.selection.targets.is_empty());
        assert!(state.selection.primary.is_none());
    }

    #[test]
    fn explicit_catalogue_request_replays_the_current_catalogue_with_correlation() {
        let mut app = command_test_app();
        let catalogue = BimClassificationFieldCatalogue {
            semantic_revision: 17,
            fields: vec![BimClassificationFieldDescriptor::new(
                BimFieldKey::property("BIM:Instance:Longueur"),
                "Longueur",
                BimPropertyScope::Instance,
            )],
        };
        app.world_mut()
            .resource_mut::<crate::viewport::bim::BimClassificationFieldCatalogueState>()
            .replace(catalogue.clone());
        let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::RequestBimClassificationFieldCatalogue {
                known_revision: Some(0),
            },
        );

        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("catalogue request must emit a response");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        assert_eq!(
            event.event,
            ViewportEvent::BimClassificationFieldCatalogueChanged { catalogue }
        );
    }

    #[test]
    fn editor_commands_author_the_live_stage_and_publish_correlation() {
        let mut app = command_test_app();
        let stage = openusd::usd::Stage::builder()
            .in_memory("bridge_editor_test.usda")
            .unwrap();
        stage
            .define_prim("/World")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        app.world_mut()
            .insert_non_send(usd_bevy::LiveStage::new(stage));

        let define_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::DefinePrim {
                path: "/World/Box".to_owned(),
                type_name: "Cube".to_owned(),
            },
        );
        app.update();

        let define_event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("define should publish an event");
        assert_eq!(
            define_event.request_id.as_deref(),
            Some(define_request.as_str())
        );
        assert!(matches!(
            define_event.event,
            ViewportEvent::EditorCommandCompleted {
                operation: EditorOperation::DefinePrim,
                ..
            }
        ));

        let attribute_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::SetAttribute {
                prim_path: "/World/Box".to_owned(),
                name: "size".to_owned(),
                type_name: "double".to_owned(),
                value: serde_json::json!(2.5),
            },
        );
        app.update();
        let attribute_event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("attribute should publish an event");
        assert_eq!(
            attribute_event.request_id.as_deref(),
            Some(attribute_request.as_str())
        );
        assert!(matches!(
            attribute_event.event,
            ViewportEvent::EditorCommandCompleted {
                operation: EditorOperation::SetAttribute,
                ..
            }
        ));

        let live = app
            .world()
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("live stage");
        assert!(usd_bevy::authoring::prim_exists(&live.stage, "/World/Box"));
        let value = live
            .stage
            .prim(openusd::sdf::path("/World/Box").unwrap())
            .attribute("size")
            .get::<openusd::sdf::Value>()
            .unwrap();
        assert!(matches!(
            value,
            Some(openusd::sdf::Value::Double(v)) if (v - 2.5).abs() < f64::EPSILON
        ));
    }
}
