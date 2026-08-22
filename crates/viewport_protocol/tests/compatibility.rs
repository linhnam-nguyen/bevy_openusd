use viewport_protocol::{
    PROTOCOL_VERSION, ViewportCommand, ViewportCommandEnvelope, ViewportEvent,
    ViewportEventEnvelope, ViewportWireMessage, decode_json_line, encode_json_line,
};

#[test]
fn viewport_command_fixture_uses_the_version_two_json_shape() {
    let message = ViewportWireMessage::Command(ViewportCommandEnvelope::new(
        "fixture-command",
        ViewportCommand::RequestSnapshot,
    ));
    let line = encode_json_line(&message).unwrap();

    assert_eq!(
        line,
        "{\"type\":\"command\",\"payload\":{\"protocol_version\":2,\"request_id\":\"fixture-command\",\"command\":{\"kind\":\"request_snapshot\"}}}\n"
    );
    assert_eq!(decode_json_line(&line).unwrap(), message);
}

#[test]
fn viewport_event_fixture_uses_the_version_two_json_shape() {
    let message = ViewportWireMessage::Event(ViewportEventEnvelope::new(
        None,
        ViewportEvent::Ready {
            protocol_version: PROTOCOL_VERSION,
        },
    ));
    let line = encode_json_line(&message).unwrap();

    assert_eq!(
        line,
        "{\"type\":\"event\",\"payload\":{\"protocol_version\":2,\"request_id\":null,\"event\":{\"kind\":\"ready\",\"payload\":{\"protocol_version\":2}}}}\n"
    );
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
