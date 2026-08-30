use viewport_protocol::{
    PROTOCOL_VERSION, ViewportCommand, ViewportCommandEnvelope, ViewportEvent,
    ViewportEventEnvelope, ViewportWireMessage, decode_json_line, encode_json_line,
};

#[test]
fn viewport_command_fixture_uses_the_current_json_shape() {
    let message = ViewportWireMessage::Command(ViewportCommandEnvelope::new(
        "fixture-command",
        ViewportCommand::RequestSnapshot,
    ));
    let line = encode_json_line(&message).unwrap();

    assert_eq!(
        line,
        "{\"type\":\"command\",\"payload\":{\"protocol_version\":7,\"request_id\":\"fixture-command\",\"command\":{\"kind\":\"request_snapshot\"}}}\n"
    );
    assert_eq!(decode_json_line(&line).unwrap(), message);
}

#[test]
fn viewport_event_fixture_uses_the_current_json_shape() {
    let message = ViewportWireMessage::Event(ViewportEventEnvelope::new(
        None,
        ViewportEvent::Ready {
            protocol_version: PROTOCOL_VERSION,
        },
    ));
    let line = encode_json_line(&message).unwrap();

    assert_eq!(
        line,
        "{\"type\":\"event\",\"payload\":{\"protocol_version\":7,\"request_id\":null,\"event\":{\"kind\":\"ready\",\"payload\":{\"protocol_version\":7}}}}\n"
    );
    assert_eq!(decode_json_line(&line).unwrap(), message);
}

#[test]
fn selection_delta_event_round_trips_through_the_v7_wire_shape() {
    let target = viewport_protocol::SceneAnchor::active_session("/World/Cube");
    let message = ViewportWireMessage::Event(ViewportEventEnvelope::new(
        Some("selection-delta".into()),
        ViewportEvent::SelectionDeltaApplied {
            revision: 3,
            added: vec![target.clone()],
            removed: Vec::new(),
            primary: Some(target),
            count: 1,
        },
    ));
    let line = encode_json_line(&message).unwrap();

    assert!(line.contains("selection_delta_applied"));
    assert!(line.contains("\"revision\":3"));
    assert_eq!(decode_json_line(&line).unwrap(), message);
}

#[test]
fn ground_grid_origin_command_round_trips_through_json() {
    let message = ViewportWireMessage::Command(ViewportCommandEnvelope::new(
        "grid-origin",
        ViewportCommand::SetGroundGridOrigin {
            origin: viewport_protocol::GroundGridOrigin::WorldOrigin,
        },
    ));
    let line = encode_json_line(&message).unwrap();

    assert!(line.contains("set_ground_grid_origin"));
    assert!(line.contains("world_origin"));
    assert_eq!(decode_json_line(&line).unwrap(), message);
}

#[test]
fn standard_views_use_the_complete_snake_case_wire_vocabulary() {
    for (view, expected) in [
        (viewport_protocol::StandardView::Front, "front"),
        (viewport_protocol::StandardView::Back, "back"),
        (viewport_protocol::StandardView::Left, "left"),
        (viewport_protocol::StandardView::Right, "right"),
        (viewport_protocol::StandardView::Top, "top"),
        (viewport_protocol::StandardView::Bottom, "bottom"),
    ] {
        assert_eq!(
            serde_json::to_string(&view).unwrap(),
            format!("\"{expected}\"")
        );
        let command = ViewportCommand::SetStandardView { view };
        let decoded: ViewportCommand =
            serde_json::from_value(serde_json::to_value(command).unwrap()).unwrap();
        assert_eq!(decoded, ViewportCommand::SetStandardView { view });
    }
}

#[test]
fn a_version_two_command_envelope_is_rejected_by_the_current_contract() {
    let mut envelope = ViewportCommandEnvelope::new("legacy", ViewportCommand::RequestSnapshot);
    envelope.protocol_version = 2;
    assert!(matches!(
        envelope.validate(),
        Err(
            viewport_protocol::ProtocolValidationError::UnsupportedProtocolVersion {
                received: 2,
                expected: 7,
            }
        )
    ));
}

#[test]
fn previous_protocol_version_is_rejected_after_hierarchy_wire_bump() {
    let mut envelope = ViewportCommandEnvelope::new("legacy-v5", ViewportCommand::RequestSnapshot);
    envelope.protocol_version = 5;
    assert!(matches!(
        envelope.validate(),
        Err(
            viewport_protocol::ProtocolValidationError::UnsupportedProtocolVersion {
                received: 5,
                expected: 7,
            }
        )
    ));
}

#[test]
fn nonfinite_camera_orientation_is_not_constructible_for_public_projection() {
    assert!(
        viewport_protocol::CameraOrientationReadModel::from_rotation_xyzw([
            f32::NAN,
            0.0,
            0.0,
            1.0,
        ])
        .is_none()
    );
}
