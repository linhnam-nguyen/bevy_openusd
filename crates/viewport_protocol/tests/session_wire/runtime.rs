use viewport_protocol::{
    AuthorizationPolicy, AuthorizedRuntimeManifest, ClientCommand, ClientCommandEnvelope,
    RuntimeBlobReference, RuntimeMutation, RuntimeMutationBatch, RuntimePayloadKind, SceneAnchor,
    SceneChildrenPage, ScenePageReference, SceneSearchMatch, SemanticSyncOperation,
    SemanticSyncPhase, SemanticSyncStatus, ServerEvent, ServerEventEnvelope, SessionCommand,
    SessionEvent, SessionId, ViewportCommand, ViewportEvent, ViewportReadModel,
    decode_client_json_line, decode_server_json_line, encode_client_json_line,
    encode_server_json_line,
};

pub(super) struct ViewportProtocolViewportSnapshot;

impl ViewportProtocolViewportSnapshot {
    pub(super) fn event() -> viewport_protocol::ViewportEvent {
        viewport_protocol::ViewportEvent::Snapshot {
            state: Box::new(ViewportReadModel {
                protocol_version: viewport_protocol::PROTOCOL_VERSION,
                stage: viewport_protocol::StageReadModel {
                    display_name: "fixture".to_owned(),
                    loaded: true,
                },
                scene: viewport_protocol::SceneReadModel::default(),
                selection: viewport_protocol::SelectionReadModel::default(),
                viewer_settings: viewport_protocol::ViewerSettingsReadModel::default(),
                camera_source: viewport_protocol::CameraSource::Arcball,
                camera_orientation: viewport_protocol::CameraOrientationReadModel::default(),
                timeline: viewport_protocol::TimelineReadModel {
                    seconds: 0.0,
                    playing: false,
                    start_time_code: 0.0,
                    end_time_code: 1.0,
                    time_codes_per_second: 24.0,
                },
                presentation: viewport_protocol::PresentationReadModel {
                    renderer: viewport_protocol::RendererConfiguration::default(),
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
            }),
        }
    }
}

#[test]
fn runtime_delivery_events_round_trip_with_blob_metadata_and_chunks() {
    let hierarchy = RuntimeBlobReference {
        blob_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        payload_kind: RuntimePayloadKind::Hierarchy,
        payload_version: 1,
        byte_size: 3,
    };
    let manifest = AuthorizedRuntimeManifest {
        revision: "working-7".to_owned(),
        profile: viewport_protocol::RuntimeProfile::NativeMedium,
        hierarchy: hierarchy.clone(),
        meshes: Vec::new(),
        materials: Vec::new(),
        textures: Vec::new(),
        redacted_blob_count: 2,
    };
    let events = [
        ServerEvent::Session(SessionEvent::RuntimeManifest { manifest }),
        ServerEvent::Session(SessionEvent::AuthorizationChanged {
            authorization: AuthorizationPolicy::default(),
        }),
        ServerEvent::Session(SessionEvent::SemanticSyncStatus {
            status: SemanticSyncStatus::phase(SemanticSyncPhase::Provisioned, None),
        }),
        ServerEvent::Session(SessionEvent::RuntimeManifestChunk {
            manifest_id: "runtime-request".to_owned(),
            chunk_index: 0,
            chunk_count: 1,
            manifest: AuthorizedRuntimeManifest {
                revision: "working-7".to_owned(),
                profile: viewport_protocol::RuntimeProfile::NativeMedium,
                hierarchy: hierarchy.clone(),
                meshes: Vec::new(),
                materials: Vec::new(),
                textures: Vec::new(),
                redacted_blob_count: 2,
            },
        }),
        ServerEvent::Session(SessionEvent::RuntimeBlobChunk {
            blob_id: hierarchy.blob_id,
            chunk_index: 0,
            chunk_count: 1,
            bytes: vec![1, 2, 3],
        }),
        ServerEvent::Session(SessionEvent::RuntimeBlobRejected {
            reason: "denied".to_owned(),
        }),
    ];

    for (sequence, event) in events.into_iter().enumerate() {
        let envelope = ServerEventEnvelope::for_request(
            SessionId::new("session-1"),
            sequence as u64 + 1,
            "runtime-request",
            event,
        );
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
        display_name: Some("Door".to_owned()),
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
                breadcrumb: node.anchor.prim_path.clone(),
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
fn client_wire_command_cannot_encode_authorization_changed() {
    let valid_client_ops = [
        SemanticSyncOperation::Provision,
        SemanticSyncOperation::Connect,
        SemanticSyncOperation::PushSnapshot,
        SemanticSyncOperation::PullProjection,
        SemanticSyncOperation::Close,
    ];
    for op in valid_client_ops {
        let cmd = ClientCommand::Session(SessionCommand::SemanticSync { operation: op });
        let envelope =
            ClientCommandEnvelope::for_session("req-1", SessionId::new("session-1"), 1, cmd);
        envelope.validate().unwrap();
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(!json.contains("authorization_changed"));
        assert!(!json.contains("AuthorizationChanged"));
    }
}

#[test]
fn semantic_sync_status_wire_event_with_approved_reason_codes_never_contains_credentials_or_urls() {
    let approved_reason_codes = [
        (SemanticSyncPhase::Failed, Some("provision_failed")),
        (SemanticSyncPhase::Failed, Some("connect_failed")),
        (SemanticSyncPhase::Failed, Some("revoke_failed")),
        (SemanticSyncPhase::Failed, Some("pull_failed")),
        (SemanticSyncPhase::Failed, Some("push_failed")),
        (SemanticSyncPhase::Failed, Some("runtime_queue_full")),
        (SemanticSyncPhase::Failed, Some("worker_unavailable")),
        (SemanticSyncPhase::Failed, Some("session_closed")),
        (SemanticSyncPhase::Stale, Some("authorization_changed")),
        (SemanticSyncPhase::Provisioning, None),
        (SemanticSyncPhase::Provisioned, None),
        (SemanticSyncPhase::Connecting, None),
        (SemanticSyncPhase::Pulling, None),
        (SemanticSyncPhase::Pushing, None),
        (SemanticSyncPhase::Closed, None),
        (SemanticSyncPhase::Disabled, None),
    ];

    for (phase, detail) in approved_reason_codes {
        let status = SemanticSyncStatus::phase(phase, detail.map(|s| s.to_owned()));
        let event = ServerEvent::Session(SessionEvent::SemanticSyncStatus { status });
        let envelope = ServerEventEnvelope::new(SessionId::new("session-1"), 1, event);
        let json = serde_json::to_string(&envelope).unwrap();

        assert!(!json.contains("token"));
        assert!(!json.contains("secret"));
        assert!(!json.contains("password"));
        assert!(!json.contains("url"));
        assert!(!json.contains("http"));
        assert!(!json.contains("database"));
        assert!(!json.contains("jwt"));
    }

    let ready_status = SemanticSyncStatus::ready("snap-123".to_owned(), "hash-abc".to_owned());
    let ready_event = ServerEvent::Session(SessionEvent::SemanticSyncStatus {
        status: ready_status,
    });
    let ready_envelope = ServerEventEnvelope::new(SessionId::new("session-1"), 2, ready_event);
    let ready_json = serde_json::to_string(&ready_envelope).unwrap();
    assert!(!ready_json.contains("token"));
    assert!(!ready_json.contains("secret"));
    assert!(!ready_json.contains("password"));
    assert!(!ready_json.contains("url"));
    assert!(!ready_json.contains("http"));
    assert!(!ready_json.contains("database"));
    assert!(!ready_json.contains("jwt"));
}
