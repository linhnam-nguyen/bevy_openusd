use viewport_protocol::{
    ActiveStreamConfiguration, ButtonState, ClientCapabilities, ClientCommand,
    ClientCommandEnvelope, ClientHello, CodecId, FocusState, HandshakeEvent, InputCommand,
    InputModifiers, KeyboardInput, PointerButtons, PointerMotion, ProtocolValidationError,
    ReleaseAllInput, RuntimeMutation, RuntimeMutationBatch, SceneAnchor, SceneChildrenPage,
    ScenePageReference, SceneSearchMatch, ServerCapabilities, ServerEvent, ServerEventEnvelope,
    ServerHello, SessionCommand, SessionEvent, SessionId, SessionRole, StreamCommand, StreamEvent,
    ViewportCommand, ViewportEvent, ViewportMetrics, ViewportReadModel, decode_client_json_line,
    decode_server_json_line, encode_client_json_line, encode_server_json_line,
};

fn metrics() -> ViewportMetrics {
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
    assert_eq!(
        serde_json::from_str::<ServerHello>(&server_json).unwrap(),
        server
    );
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
        ClientCommand::Session(SessionCommand::Ping {
            nonce: "ping-1".to_owned(),
        }),
        ClientCommand::Stream(StreamCommand::ConfigureViewport { metrics: metrics() }),
        ClientCommand::Input(InputCommand::PointerMotion(PointerMotion {
            sequence: 7,
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
fn lazy_scene_queries_round_trip_with_request_correlation() {
    let world = SceneAnchor::active_session("/World");
    let command = ClientCommandEnvelope::for_session(
        "expand-1",
        SessionId::new("session-1"),
        9,
        ClientCommand::Viewport(ViewportCommand::RequestSceneChildren {
            parent: Some(world.clone()),
            page: 2,
            page_size: 64,
        }),
    );
    assert_eq!(
        decode_client_json_line(&encode_client_json_line(&command).unwrap()).unwrap(),
        command
    );

    let node = viewport_protocol::PrimNodeReadModel {
        anchor: SceneAnchor::active_session("/World/Door"),
        parent: Some(world.clone()),
        label: "Door".to_owned(),
        visible: true,
        has_children: false,
    };
    let event = ServerEventEnvelope::for_request(
        SessionId::new("session-1"),
        12,
        "search-1",
        ServerEvent::Viewport(viewport_protocol::ViewportEvent::SearchResults {
            query: "door".to_owned(),
            offset: 0,
            total: 1,
            matches: vec![SceneSearchMatch {
                anchor: node.anchor.clone(),
                parent: node.parent.clone(),
                label: node.label.clone(),
                visible: node.visible,
                has_children: node.has_children,
                reveal_pages: vec![ScenePageReference {
                    parent: Some(world.clone()),
                    page: 0,
                }],
            }],
            has_more: false,
        }),
    );
    let decoded = decode_server_json_line(&encode_server_json_line(&event).unwrap()).unwrap();
    assert_eq!(decoded, event);

    let page = SceneChildrenPage {
        parent: Some(world),
        page: 0,
        page_size: 64,
        total: 1,
        nodes: vec![node],
    };
    assert_eq!(page.nodes.len(), 1);
}

#[test]
fn editor_commands_and_events_round_trip_with_frontend_values() {
    let command = ClientCommandEnvelope::for_session(
        "edit-1",
        SessionId::new("session-1"),
        10,
        ClientCommand::Viewport(ViewportCommand::SetAttribute {
            prim_path: "/World/Box".to_owned(),
            name: "size".to_owned(),
            type_name: "double".to_owned(),
            value: serde_json::json!(2.5),
        }),
    );
    command.validate().unwrap();
    assert_eq!(
        decode_client_json_line(&encode_client_json_line(&command).unwrap()).unwrap(),
        command
    );

    let event = ServerEventEnvelope::for_request(
        SessionId::new("session-1"),
        13,
        "edit-1",
        ServerEvent::Viewport(viewport_protocol::ViewportEvent::EditorCommandCompleted {
            operation: viewport_protocol::EditorOperation::SetAttribute,
            changed_paths: vec!["/World/Box.size".to_owned()],
            state: viewport_protocol::EditorStateReadModel {
                can_undo: true,
                can_redo: false,
            },
        }),
    );
    assert_eq!(
        decode_server_json_line(&encode_server_json_line(&event).unwrap()).unwrap(),
        event
    );
}

#[test]
fn runtime_mutation_batch_round_trips_and_validates() {
    let batch = RuntimeMutationBatch {
        source_id: "revit-connector".to_owned(),
        sequence: 8,
        base_revision: 4,
        operations: vec![RuntimeMutation::SetAttribute {
            prim_path: "/World/Box".to_owned(),
            name: "Comments".to_owned(),
            type_name: "string".to_owned(),
            value: serde_json::json!("external edit"),
        }],
    };
    let command = ClientCommandEnvelope::for_session(
        "runtime-8",
        SessionId::new("session-1"),
        14,
        ClientCommand::Viewport(ViewportCommand::ApplyRuntimeMutationBatch {
            batch: batch.clone(),
        }),
    );
    command.validate().unwrap();
    assert_eq!(
        decode_client_json_line(&encode_client_json_line(&command).unwrap()).unwrap(),
        command
    );

    let event = ServerEventEnvelope::for_request(
        SessionId::new("session-1"),
        15,
        "runtime-8",
        ServerEvent::Viewport(ViewportEvent::RuntimeMutationBatchAccepted {
            source_id: batch.source_id,
            sequence: batch.sequence,
            base_revision: batch.base_revision,
            applied_operations: 1,
            changed_paths: vec!["/World/Box.Comments".to_owned()],
            state: viewport_protocol::EditorStateReadModel::default(),
        }),
    );
    assert_eq!(
        decode_server_json_line(&encode_server_json_line(&event).unwrap()).unwrap(),
        event
    );
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

struct ViewportProtocolViewportSnapshot;

impl ViewportProtocolViewportSnapshot {
    fn event() -> viewport_protocol::ViewportEvent {
        viewport_protocol::ViewportEvent::Snapshot {
            state: ViewportReadModel {
                protocol_version: viewport_protocol::PROTOCOL_VERSION,
                stage: viewport_protocol::StageReadModel {
                    display_name: "fixture".to_owned(),
                    loaded: true,
                },
                scene: viewport_protocol::SceneReadModel::default(),
                selection: viewport_protocol::SelectionReadModel { target: None },
                camera_source: viewport_protocol::CameraSource::Arcball,
                timeline: viewport_protocol::TimelineReadModel {
                    seconds: 0.0,
                    playing: false,
                    start_time_code: 0.0,
                    end_time_code: 1.0,
                    time_codes_per_second: 24.0,
                },
                presentation: viewport_protocol::PresentationReadModel {
                    ground_grid: true,
                    ground_grid_origin: viewport_protocol::GroundGridOrigin::LoadedScene,
                    world_axes: true,
                    prim_markers: true,
                    prim_marker_bias: 0.0,
                    skeleton: true,
                    physics: false,
                    colliders: false,
                    wireframe: false,
                    light_intensity_scale: 1.0,
                    curve_tuning: viewport_protocol::CurveTuning {
                        default_radius: 0.01,
                        ring_segments: 8,
                        point_scale: 1.0,
                    },
                },
                physics_running: false,
            },
        }
    }
}
