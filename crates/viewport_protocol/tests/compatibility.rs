use viewport_protocol::{
    PROTOCOL_VERSION, ViewportCommand, ViewportCommandEnvelope, ViewportEvent,
    ViewportEventEnvelope, ViewportWireMessage, decode_json_line, encode_json_line,
};

#[test]
fn legacy_command_fixture_keeps_the_version_one_json_shape() {
    let message = ViewportWireMessage::Command(ViewportCommandEnvelope::new(
        "fixture-command",
        ViewportCommand::RequestSnapshot,
    ));
    let line = encode_json_line(&message).unwrap();

    assert_eq!(
        line,
        "{\"type\":\"command\",\"payload\":{\"protocol_version\":1,\"request_id\":\"fixture-command\",\"command\":{\"kind\":\"request_snapshot\"}}}\n"
    );
    assert_eq!(decode_json_line(&line).unwrap(), message);
}

#[test]
fn legacy_event_fixture_keeps_the_version_one_json_shape() {
    let message = ViewportWireMessage::Event(ViewportEventEnvelope::new(
        None,
        ViewportEvent::Ready {
            protocol_version: PROTOCOL_VERSION,
        },
    ));
    let line = encode_json_line(&message).unwrap();

    assert_eq!(
        line,
        "{\"type\":\"event\",\"payload\":{\"protocol_version\":1,\"request_id\":null,\"event\":{\"kind\":\"ready\",\"payload\":{\"protocol_version\":1}}}}\n"
    );
    assert_eq!(decode_json_line(&line).unwrap(), message);
}

