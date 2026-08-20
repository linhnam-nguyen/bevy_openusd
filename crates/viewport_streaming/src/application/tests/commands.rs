use crate::application::interface::RenderServerInterface;
use crate::application::types::RenderServerPortError;
use viewport_protocol::{
    InputCommand, PointerMotion, ViewportCommand, ViewportCommandEnvelope, ViewportEvent,
    ViewportEventEnvelope, ViewportMetrics, ViewportReadModel,
};

#[test]
fn stream_generation_remains_monotonic_across_webview_reconnects() {
    let interface = RenderServerInterface::default();

    let first = interface
        .submit_stream_configuration(ViewportMetrics::default())
        .expect("initial configuration should be valid");
    assert_eq!(first.generation, 1);
    assert_eq!(interface.take_stream_configuration(), Some(first));

    let resized = interface
        .submit_stream_configuration(ViewportMetrics {
            generation: 2,
            ..ViewportMetrics::default()
        })
        .expect("resized configuration should be valid");
    assert_eq!(resized.generation, 2);
    assert_eq!(interface.take_stream_configuration(), Some(resized));

    // A Tauri WebView refresh starts its local counter at one again. The
    // server must advance from the prior resize instead of reusing one.
    let refreshed = interface
        .submit_stream_configuration(ViewportMetrics::default())
        .expect("reconnected configuration should be valid");
    assert_eq!(refreshed.generation, 3);
    assert_eq!(interface.take_stream_configuration(), Some(refreshed));
}

#[test]
fn snapshot_display_name_is_reduced_to_a_basename() {
    let interface = RenderServerInterface::default();
    interface
        .publish_viewport_event(ViewportEventEnvelope::new(
            None,
            ViewportEvent::Snapshot {
                state: ViewportReadModel::unloaded("/private/stages/Kitchen_set.usdz"),
            },
        ))
        .unwrap();

    assert_eq!(
        interface
            .take_latest_snapshot(ViewportReadModel::unloaded("fallback.usda"))
            .stage
            .display_name,
        "Kitchen_set.usdz"
    );
}

#[test]
fn event_history_is_cleared_when_a_session_takes_a_snapshot() {
    let interface = RenderServerInterface::default();
    let snapshot = ViewportReadModel::unloaded("stage.usda");
    interface
        .publish_viewport_event(ViewportEventEnvelope::new(
            None,
            ViewportEvent::Snapshot {
                state: snapshot.clone(),
            },
        ))
        .unwrap();

    assert_eq!(interface.pending_event_count(), 1);
    assert_eq!(interface.take_latest_snapshot(snapshot.clone()), snapshot);
    assert_eq!(interface.pending_event_count(), 0);
}

#[test]
fn viewport_command_queue_rejects_invalid_correlation_metadata() {
    let interface = RenderServerInterface::default();
    let command = ViewportCommandEnvelope::new("", ViewportCommand::RequestSnapshot);

    assert_eq!(
        interface.submit_viewport_command(command),
        Err(RenderServerPortError::InvalidPayload)
    );
    assert_eq!(interface.pending_command_count(), 0);
}

#[test]
fn an_event_can_be_put_back_after_a_failed_transport_send() {
    let interface = RenderServerInterface::default();
    let event = ViewportEventEnvelope::new(
        Some("request-1".to_owned()),
        ViewportEvent::PhysicsChanged { running: true },
    );
    interface
        .publish_viewport_event(event.clone())
        .expect("event should enter the bounded queue");
    let removed = interface
        .pop_viewport_event()
        .expect("event should be available for transport");
    interface
        .requeue_viewport_event_front(removed)
        .expect("failed sends must be recoverable");

    assert_eq!(interface.pop_viewport_event(), Some(event));
}

#[test]
fn pointer_motion_keeps_only_the_newest_valid_packet() {
    let interface = RenderServerInterface::default();
    let motion = |sequence| PointerMotion {
        sequence,
        dx_css_pixels: sequence as f32,
        dy_css_pixels: 0.0,
        wheel_x: 0.0,
        wheel_y: 0.0,
        viewport_css_width: 800.0,
        viewport_css_height: 600.0,
        stream_generation: 0,
    };

    interface
        .submit_input(InputCommand::PointerMotion(motion(1)))
        .unwrap();
    interface
        .submit_input(InputCommand::PointerMotion(motion(2)))
        .unwrap();
    interface
        .submit_input(InputCommand::PointerMotion(motion(1)))
        .unwrap();

    assert_eq!(interface.take_latest_pointer_motion().unwrap().sequence, 2);
    assert!(interface.take_latest_pointer_motion().is_none());
}

#[test]
fn reliable_input_state_is_preserved_in_order() {
    let interface = RenderServerInterface::default();
    interface
        .submit_input(InputCommand::ReleaseAll(
            viewport_protocol::ReleaseAllInput { sequence: 1 },
        ))
        .unwrap();
    assert!(matches!(
        interface.pop_input(),
        Some(InputCommand::ReleaseAll(_))
    ));
}
