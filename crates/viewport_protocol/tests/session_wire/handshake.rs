use viewport_protocol::{
    ActiveStreamConfiguration, AuthorizationPolicy, ButtonState, ClientCapabilities, ClientCommand,
    ClientCommandEnvelope, ClientHello, CodecId, DeliveryMode, FocusState, HandshakeEvent,
    HistoryPermission, InputCommand, InputModifiers, KeyboardInput, ModelDownloadPermission,
    PointerButtons, PointerMotion, ProtocolValidationError, ReleaseAllInput, SemanticPropertyScope,
    ServerCapabilities, ServerEvent, ServerEventEnvelope, ServerHello, SessionCommand,
    SessionEvent, SessionId, SessionRole, StreamCommand, StreamEvent, ViewportCommand,
    ViewportMetrics, decode_client_json_line, decode_server_json_line, encode_client_json_line,
    encode_server_json_line,
};

use super::runtime::ViewportProtocolViewportSnapshot;

pub(super) fn metrics() -> ViewportMetrics {
    ViewportMetrics {
        css_width: 1280,
        css_height: 720,
        device_pixel_ratio: 2.0,
        requested_width: 2560,
        requested_height: 1440,
        preferred_fps: Some(60),
        generation: 4,
    }
}

#[test]
fn handshake_and_capabilities_round_trip() {
    let hello = ClientHello::new("desktop-1", ClientCapabilities::default());
    hello.validate().unwrap();
    let message = HandshakeEvent::ClientHello(hello.clone());

    let json = serde_json::to_string(&message).unwrap();
    let decoded: HandshakeEvent = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded, message);
    assert_eq!(hello.capabilities.codecs, vec![CodecId::H264]);

    let server = ServerHello::new(
        SessionId::new("session-1"),
        SessionRole::Controller,
        ServerCapabilities::default(),
    );
    server.validate().unwrap();
    let server_json = serde_json::to_string(&server).unwrap();
    assert!(!server_json.contains("\"authorization\""));
    assert_eq!(
        serde_json::from_str::<ServerHello>(&server_json).unwrap(),
        server
    );
}

#[test]
fn authorization_policy_round_trips_separately_from_capabilities() {
    let policy = AuthorizationPolicy {
        allowed_delivery_modes: vec![DeliveryMode::Stream],
        model_download: ModelDownloadPermission::Denied,
        allowed_blob_ids: Vec::new(),
        semantic_property_scope: SemanticPropertyScope::AllowList(vec!["displayName".to_owned()]),
        history: HistoryPermission::ReadOnly,
        runtime_profile: viewport_protocol::RuntimeProfile::ServerStream,
    };
    let server = ServerHello::with_authorization(
        SessionId::new("session-policy"),
        SessionRole::Observer,
        ServerCapabilities::default(),
        policy.clone(),
    );

    server.validate().unwrap();
    let json = serde_json::to_string(&server).unwrap();
    let decoded: ServerHello = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded.authorization, policy);
    assert!(json.contains("authorization"));
    assert_eq!(decoded.capabilities, ServerCapabilities::default());
}

#[test]
fn application_handshake_uses_the_versioned_command_and_event_envelopes() {
    let hello = ClientHello::new("browser-1", ClientCapabilities::default());
    let command = ClientCommandEnvelope::new("handshake-1", 1, ClientCommand::Handshake(hello));
    assert!(command.session_id.is_none());
    assert_eq!(
        decode_client_json_line(&encode_client_json_line(&command).unwrap()).unwrap(),
        command
    );

    let session_id = SessionId::new("session-1");
    let event = ServerEventEnvelope::new(
        session_id.clone(),
        1,
        ServerEvent::Handshake(HandshakeEvent::Ready { session_id }),
    );
    assert_eq!(
        decode_server_json_line(&encode_server_json_line(&event).unwrap()).unwrap(),
        event
    );
}

#[test]
fn every_client_command_family_round_trips_through_a_client_envelope() {
    let commands = [
        ClientCommand::Session(SessionCommand::RequestRuntimeManifest),
        ClientCommand::Session(SessionCommand::RequestRuntimeBlob {
            blob_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        }),
        ClientCommand::Session(SessionCommand::SemanticSync {
            operation: viewport_protocol::SemanticSyncOperation::Provision,
        }),
        ClientCommand::Session(SessionCommand::Ping {
            nonce: "ping-1".to_owned(),
        }),
        ClientCommand::Stream(StreamCommand::ConfigureViewport { metrics: metrics() }),
        ClientCommand::Input(InputCommand::PointerMotion(PointerMotion {
            sequence: 7,
            x_css_pixels: 640.0,
            y_css_pixels: 360.0,
            dx_css_pixels: 2.5,
            dy_css_pixels: -1.0,
            wheel_x: 0.0,
            wheel_y: -3.0,
            viewport_css_width: 1280.0,
            viewport_css_height: 720.0,
            stream_generation: 4,
        })),
        ClientCommand::Viewport(ViewportCommand::RequestSnapshot),
    ];

    for (sequence, command) in commands.into_iter().enumerate() {
        let envelope = ClientCommandEnvelope::for_session(
            format!("request-{sequence}"),
            SessionId::new("session-1"),
            sequence as u64 + 1,
            command,
        );
        envelope.validate().unwrap();
        let line = encode_client_json_line(&envelope).unwrap();
        assert_eq!(decode_client_json_line(&line).unwrap(), envelope);
    }
}

#[test]
fn every_server_event_family_round_trips_through_a_server_envelope() {
    let events = [
        ServerEvent::Session(SessionEvent::Pong {
            nonce: "ping-1".to_owned(),
        }),
        ServerEvent::Stream(StreamEvent::ConfigurationApplied {
            configuration: ActiveStreamConfiguration {
                width: 1280,
                height: 720,
                fps: 60,
                codec: CodecId::H264,
                generation: 4,
            },
        }),
        ServerEvent::Viewport(ViewportProtocolViewportSnapshot::event()),
    ];

    for (sequence, event) in events.into_iter().enumerate() {
        let envelope =
            ServerEventEnvelope::new(SessionId::new("session-1"), sequence as u64 + 1, event);
        envelope.validate().unwrap();
        let line = encode_server_json_line(&envelope).unwrap();
        assert_eq!(decode_server_json_line(&line).unwrap(), envelope);
    }
}

#[test]
fn input_types_are_independently_serializable() {
    let input = InputCommand::ButtonState(ButtonState {
        sequence: 2,
        buttons: PointerButtons {
            primary: true,
            secondary: false,
            auxiliary: false,
        },
        modifiers: InputModifiers {
            shift: true,
            control: false,
            alt: false,
            meta: false,
        },
        stream_generation: 1,
    });
    let keyboard = InputCommand::Keyboard(KeyboardInput {
        sequence: 3,
        code: "KeyW".to_owned(),
        key: Some("w".to_owned()),
        pressed: true,
        repeat: false,
        modifiers: InputModifiers::default(),
        stream_generation: 1,
    });
    let focus = InputCommand::FocusChanged(FocusState {
        focused: false,
        sequence: 4,
    });
    let release = InputCommand::ReleaseAll(ReleaseAllInput { sequence: 5 });

    for value in [input, keyboard, focus, release] {
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<InputCommand>(&json).unwrap(), value);
    }
}

#[test]
fn validation_rejects_versions_and_numeric_boundaries() {
    let mut envelope = ClientCommandEnvelope::new(
        "request-1",
        1,
        ClientCommand::Stream(StreamCommand::ConfigureViewport { metrics: metrics() }),
    );
    envelope.protocol_version += 1;
    assert!(matches!(
        envelope.validate(),
        Err(ProtocolValidationError::UnsupportedProtocolVersion { .. })
    ));

    let mut invalid = metrics();
    invalid.requested_width = 1279;
    assert!(matches!(
        invalid.validate(),
        Err(ProtocolValidationError::OddEncodedDimension { .. })
    ));

    invalid = metrics();
    invalid.device_pixel_ratio = f32::INFINITY;
    assert!(matches!(
        invalid.validate(),
        Err(ProtocolValidationError::InvalidDevicePixelRatio { .. })
    ));

    invalid = metrics();
    invalid.preferred_fps = Some(241);
    assert!(matches!(
        invalid.validate(),
        Err(ProtocolValidationError::InvalidFrameRate { .. })
    ));
}

#[test]
fn validation_rejects_unbounded_input_motion() {
    let envelope = ClientCommandEnvelope::new(
        "input-1",
        1,
        ClientCommand::Input(InputCommand::PointerMotion(PointerMotion {
            sequence: 1,
            x_css_pixels: 640.0,
            y_css_pixels: 360.0,
            dx_css_pixels: 4097.0,
            dy_css_pixels: 0.0,
            wheel_x: 0.0,
            wheel_y: 0.0,
            viewport_css_width: 1280.0,
            viewport_css_height: 720.0,
            stream_generation: 0,
        })),
    );

    assert!(matches!(
        envelope.validate(),
        Err(ProtocolValidationError::InvalidInput {
            field: "pointer.dx_css_pixels"
        })
    ));
}

#[test]
fn constructors_reserve_deterministic_request_session_and_sequence_metadata() {
    let command = ClientCommandEnvelope::for_session(
        "request-9",
        SessionId::new("session-2"),
        9,
        ClientCommand::Session(SessionCommand::RequestSnapshot),
    );
    assert_eq!(
        command.protocol_version,
        viewport_protocol::PROTOCOL_VERSION
    );
    assert_eq!(command.request_id, "request-9");
    assert_eq!(command.sequence, 9);
    assert_eq!(command.session_id, Some(SessionId::new("session-2")));

    let event = ServerEventEnvelope::for_request(
        SessionId::new("session-2"),
        10,
        "request-9",
        ServerEvent::Session(SessionEvent::Ready {
            snapshot_required: true,
        }),
    );
    assert_eq!(event.request_id.as_deref(), Some("request-9"));
    assert_eq!(event.sequence, 10);
}
