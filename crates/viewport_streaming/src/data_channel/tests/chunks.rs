use viewport_protocol::{
    CameraSource, ServerEvent, SessionEvent, SessionId, ViewportEvent, ViewportReadModel,
};

use crate::application::RenderServerInterface;
use crate::data_channel::chunks::queue_runtime_blob;
use crate::data_channel::constants::{
    MAX_APPLICATION_MESSAGE_BYTES, MAX_COMPACT_STAGE_DISPLAY_NAME_CHARS,
};
use crate::data_channel::dispatch::encoded_size;
use crate::data_channel::events::queue_server_event_for_request;
use crate::data_channel::session::ApplicationSession;

#[test]
fn large_snapshots_are_chunked_into_bounded_ordered_events() {
    let session = ApplicationSession::new(
        SessionId::new("session-1"),
        ViewportReadModel::unloaded("stage.usda"),
        RenderServerInterface::default(),
    );
    let mut state = session.state.lock().unwrap();
    let mut snapshot = ViewportReadModel::unloaded("stage.usda");
    snapshot.stage.loaded = true;
    snapshot.scene.prims = (0..2_745)
        .map(|index| viewport_protocol::PrimNodeReadModel {
            anchor: viewport_protocol::SceneAnchor::active_session(format!("/World/Prim{index}")),
            parent: None,
            label: format!("Prim {index}"),
            display_name: None,
            visible: true,
            has_children: false,
        })
        .collect();

    queue_server_event_for_request(
        &mut state,
        Some("snapshot-request".to_owned()),
        ServerEvent::Session(SessionEvent::Snapshot { state: snapshot }),
    );

    assert!(state.pending_server_events.len() > 1);
    let mut expected_count = None;
    for (sequence_index, envelope) in state.pending_server_events.iter().enumerate() {
        assert_eq!(envelope.sequence, sequence_index as u64 + 1);
        let ServerEvent::Session(SessionEvent::SnapshotChunk {
            snapshot_id,
            chunk_index,
            chunk_count,
            ..
        }) = &envelope.event
        else {
            panic!("large snapshots must be sent as snapshot chunks");
        };
        assert_eq!(snapshot_id, "snapshot-request");
        assert_eq!(*chunk_index, sequence_index as u32);
        expected_count.get_or_insert(*chunk_count);
        assert_eq!(expected_count, Some(*chunk_count));
        assert!(encoded_size(envelope).unwrap() <= MAX_APPLICATION_MESSAGE_BYTES);
    }
    assert_eq!(
        expected_count,
        Some(state.pending_server_events.len() as u32)
    );
}

#[test]
fn terminally_oversized_prim_is_omitted_from_bounded_snapshot_chunks() {
    let session = ApplicationSession::new(
        SessionId::new("session-1"),
        ViewportReadModel::unloaded("stage.usda"),
        RenderServerInterface::default(),
    );
    let mut state = session.state.lock().unwrap();
    let mut snapshot = ViewportReadModel::unloaded("stage.usda");
    snapshot.stage.loaded = true;
    snapshot.scene.total_prims = 3;
    snapshot.scene.total_roots = 3;
    snapshot.scene.root_page_size = 64;
    snapshot.scene.prims = vec![
        viewport_protocol::PrimNodeReadModel {
            anchor: viewport_protocol::SceneAnchor::active_session("/World/KeptA"),
            parent: None,
            label: "Kept A".to_owned(),
            display_name: None,
            visible: true,
            has_children: false,
        },
        viewport_protocol::PrimNodeReadModel {
            anchor: viewport_protocol::SceneAnchor::active_session(format!(
                "/World/{}",
                "x".repeat(MAX_APPLICATION_MESSAGE_BYTES)
            )),
            parent: None,
            label: "Too large".to_owned(),
            display_name: None,
            visible: true,
            has_children: false,
        },
        viewport_protocol::PrimNodeReadModel {
            anchor: viewport_protocol::SceneAnchor::active_session("/World/KeptB"),
            parent: None,
            label: "Kept B".to_owned(),
            display_name: None,
            visible: true,
            has_children: false,
        },
    ];

    queue_server_event_for_request(
        &mut state,
        Some("snapshot-request".to_owned()),
        ServerEvent::Session(SessionEvent::Snapshot { state: snapshot }),
    );

    let mut labels = Vec::new();
    for envelope in &state.pending_server_events {
        assert!(encoded_size(envelope).unwrap() <= MAX_APPLICATION_MESSAGE_BYTES);
        let ServerEvent::Session(SessionEvent::SnapshotChunk { state, .. }) = &envelope.event
        else {
            panic!("remaining snapshot nodes must stay in bounded chunks");
        };
        labels.extend(state.scene.prims.iter().map(|prim| prim.label.as_str()));
    }
    assert_eq!(labels, ["Kept A", "Kept B"]);
}

#[test]
fn oversized_snapshot_metadata_is_compacted_to_a_bounded_snapshot() {
    let session = ApplicationSession::new(
        SessionId::new("session-1"),
        ViewportReadModel::unloaded("stage.usda"),
        RenderServerInterface::default(),
    );
    let mut state = session.state.lock().unwrap();
    let oversized = "x".repeat(MAX_APPLICATION_MESSAGE_BYTES);
    let mut snapshot = ViewportReadModel::unloaded(oversized.clone());
    snapshot.stage.loaded = true;
    snapshot.scene.total_prims = 2_745;
    snapshot.scene.total_roots = 64;
    snapshot.scene.root_page_size = 64;
    let selected_target = viewport_protocol::SceneAnchor::active_session(oversized.clone());
    snapshot.selection = viewport_protocol::SelectionReadModel {
        targets: vec![selected_target.clone()],
        primary: Some(selected_target),
    };
    snapshot.camera_source = CameraSource::Authored {
        prim_path: oversized,
    };

    queue_server_event_for_request(
        &mut state,
        Some("snapshot-request".to_owned()),
        ServerEvent::Session(SessionEvent::Snapshot { state: snapshot }),
    );

    assert_eq!(state.pending_server_events.len(), 1);
    let envelope = state.pending_server_events.front().unwrap();
    assert_eq!(envelope.sequence, 1);
    assert!(encoded_size(envelope).unwrap() <= MAX_APPLICATION_MESSAGE_BYTES);
    let ServerEvent::Session(SessionEvent::Snapshot { state }) = &envelope.event else {
        panic!("oversized metadata must become one bounded snapshot");
    };
    assert!(state.scene.prims.is_empty());
    assert_eq!(state.scene.total_prims, 2_745);
    assert!(state.selection.targets.is_empty());
    assert!(state.selection.primary.is_none());
    assert_eq!(state.camera_source, CameraSource::Arcball);
    assert!(state.stage.display_name.chars().count() <= MAX_COMPACT_STAGE_DISPLAY_NAME_CHARS + 1);
}

#[test]
fn oversized_scene_child_pages_are_split_without_changing_page_identity() {
    let session = ApplicationSession::new(
        SessionId::new("session-1"),
        ViewportReadModel::unloaded("stage.usda"),
        RenderServerInterface::default(),
    );
    let mut state = session.state.lock().unwrap();
    let parent =
        viewport_protocol::SceneAnchor::active_session("/World/Very/Deep/Animated/Geometry/geom");
    let nodes = (0..128)
        .map(|index| viewport_protocol::PrimNodeReadModel {
            anchor: viewport_protocol::SceneAnchor::active_session(format!(
                "/World/Very/Deep/Animated/Geometry/geom/child-{index}-with-a-long-name"
            )),
            parent: Some(parent.clone()),
            label: format!("child-{index}"),
            display_name: None,
            visible: true,
            has_children: false,
        })
        .collect();

    queue_server_event_for_request(
        &mut state,
        Some("children-request".to_owned()),
        ServerEvent::Viewport(ViewportEvent::SceneChildren {
            page: viewport_protocol::SceneChildrenPage {
                parent: Some(parent.clone()),
                page: 0,
                page_size: viewport_protocol::DEFAULT_SCENE_PAGE_SIZE,
                total: 128,
                nodes,
            },
        }),
    );

    assert!(state.pending_server_events.len() > 1);
    let mut received_nodes = 0;
    for (sequence_index, envelope) in state.pending_server_events.iter().enumerate() {
        assert_eq!(envelope.sequence, sequence_index as u64 + 1);
        assert!(encoded_size(envelope).unwrap() <= MAX_APPLICATION_MESSAGE_BYTES);
        let ServerEvent::Viewport(ViewportEvent::SceneChildren { page }) = &envelope.event else {
            panic!("oversized child pages must remain child-page events");
        };
        assert_eq!(page.parent.as_ref(), Some(&parent));
        assert_eq!(page.page, 0);
        assert_eq!(page.page_size, viewport_protocol::DEFAULT_SCENE_PAGE_SIZE);
        assert_eq!(page.total, 128);
        received_nodes += page.nodes.len();
    }
    assert_eq!(received_nodes, 128);
}

#[test]
fn oversized_bim_property_results_are_paged_without_loss_or_duplication() {
    let session = ApplicationSession::new(
        SessionId::new("session-1"),
        ViewportReadModel::unloaded("stage.usda"),
        RenderServerInterface::default(),
    );
    let mut state = session.state.lock().unwrap();
    let target = viewport_protocol::SceneAnchor::active_session("/World/Window");
    let properties = viewport_protocol::BimPropertiesReadModel {
        targets: vec![target],
        selection_revision: 11,
        groups: vec![viewport_protocol::BimPropertyGroupReadModel {
            id: viewport_protocol::BimPropertyGroupId::Semantic,
            name: "Semantic".to_owned(),
            editable_group: true,
            properties: (0..40)
                .map(|index| viewport_protocol::BimPropertyReadModel {
                    key: format!("Property-{index}"),
                    group_id: viewport_protocol::BimPropertyGroupId::Semantic,
                    value: viewport_protocol::CommonValue::Same(
                        viewport_protocol::CanonicalValue::Text("x".repeat(400)),
                    ),
                    target_values: vec![viewport_protocol::CanonicalValue::Text("x".repeat(400))],
                    measurement: None,
                    units: Vec::new(),
                    current_display_unit: None,
                    editable: true,
                })
                .collect(),
        }],
    };

    queue_server_event_for_request(
        &mut state,
        Some("bim-properties".to_owned()),
        ServerEvent::Viewport(ViewportEvent::BimPropertiesRead {
            properties,
            diff: None,
        }),
    );

    assert!(state.pending_server_events.len() > 1);
    let expected_page_count = state.pending_server_events.len() as u32;
    let mut keys = std::collections::BTreeSet::new();
    for (sequence_index, envelope) in state.pending_server_events.iter().enumerate() {
        assert_eq!(envelope.sequence, sequence_index as u64 + 1);
        assert_eq!(envelope.request_id.as_deref(), Some("bim-properties"));
        assert!(encoded_size(envelope).unwrap() <= MAX_APPLICATION_MESSAGE_BYTES);
        let ServerEvent::Viewport(ViewportEvent::BimPropertiesPage { page }) = &envelope.event
        else {
            panic!("oversized BIM properties must be sent as property pages");
        };
        assert_eq!(page.page_index, sequence_index as u32);
        assert_eq!(page.page_count, expected_page_count);
        assert_eq!(page.total_properties, 40);
        for property in page.groups.iter().flat_map(|group| &group.properties) {
            assert!(
                keys.insert(property.key.clone()),
                "duplicate property page item"
            );
        }
    }
    assert_eq!(keys.len(), 40);
}

#[test]
fn individually_oversized_bim_property_becomes_explicit_delivery_error() {
    let session = ApplicationSession::new(
        SessionId::new("session-1"),
        ViewportReadModel::unloaded("stage.usda"),
        RenderServerInterface::default(),
    );
    let mut state = session.state.lock().unwrap();
    let oversized = viewport_protocol::BimPropertiesReadModel {
        targets: vec![viewport_protocol::SceneAnchor::active_session(
            "/World/Window",
        )],
        selection_revision: 12,
        groups: vec![viewport_protocol::BimPropertyGroupReadModel {
            id: viewport_protocol::BimPropertyGroupId::Semantic,
            name: "Semantic".to_owned(),
            editable_group: true,
            properties: vec![viewport_protocol::BimPropertyReadModel {
                key: "Oversized".to_owned(),
                group_id: viewport_protocol::BimPropertyGroupId::Semantic,
                value: viewport_protocol::CommonValue::Same(
                    viewport_protocol::CanonicalValue::Text(
                        "x".repeat(MAX_APPLICATION_MESSAGE_BYTES * 2),
                    ),
                ),
                target_values: vec![viewport_protocol::CanonicalValue::Text(
                    "x".repeat(MAX_APPLICATION_MESSAGE_BYTES * 2),
                )],
                measurement: None,
                units: Vec::new(),
                current_display_unit: None,
                editable: true,
            }],
        }],
    };

    queue_server_event_for_request(
        &mut state,
        Some("bim-error".to_owned()),
        ServerEvent::Viewport(ViewportEvent::BimPropertiesRead {
            properties: oversized,
            diff: None,
        }),
    );

    assert_eq!(state.pending_server_events.len(), 1);
    let envelope = state.pending_server_events.front().unwrap();
    assert!(encoded_size(envelope).unwrap() <= MAX_APPLICATION_MESSAGE_BYTES);
    let ServerEvent::Viewport(ViewportEvent::BimPropertiesError { error }) = &envelope.event else {
        panic!("oversized BIM property must produce an explicit error");
    };
    assert_eq!(error.selection_revision, 12);
    assert_eq!(
        error.kind,
        viewport_protocol::BimPropertiesDeliveryErrorKind::OversizedPropertyValue
    );
    assert_eq!(error.property.as_deref(), Some("Oversized"));
}

#[test]
fn runtime_blob_chunks_are_bounded_and_keep_ordered_sequences() {
    let session = ApplicationSession::new(
        SessionId::new("session-1"),
        ViewportReadModel::unloaded("stage.usda"),
        RenderServerInterface::default(),
    );
    let mut state = session.state.lock().unwrap();
    let blob_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bytes = (0..10_000)
        .map(|value| (value % 251) as u8)
        .collect::<Vec<_>>();

    queue_runtime_blob(
        &mut state,
        Some("runtime-blob"),
        blob_id.to_owned(),
        bytes.clone(),
    );

    let mut reconstructed: Vec<u8> = Vec::new();
    let expected_chunk_count = state.pending_server_events.len() as u32;
    for (sequence_index, envelope) in state.pending_server_events.iter().enumerate() {
        assert_eq!(envelope.sequence, sequence_index as u64 + 1);
        assert!(encoded_size(envelope).unwrap() <= MAX_APPLICATION_MESSAGE_BYTES);
        let ServerEvent::Session(SessionEvent::RuntimeBlobChunk {
            blob_id: event_blob_id,
            chunk_index,
            chunk_count,
            bytes,
        }) = &envelope.event
        else {
            panic!("runtime blob must be sent as blob chunks");
        };
        assert_eq!(event_blob_id, blob_id);
        assert_eq!(*chunk_index, sequence_index as u32);
        assert_eq!(*chunk_count, expected_chunk_count);
        reconstructed.extend(bytes);
    }

    assert_eq!(reconstructed, bytes);
    assert_eq!(
        expected_chunk_count,
        state.pending_server_events.len() as u32
    );
}

#[test]
fn oversized_runtime_manifests_are_split_without_leaking_unbounded_events() {
    let session = ApplicationSession::new(
        SessionId::new("session-1"),
        ViewportReadModel::unloaded("stage.usda"),
        RenderServerInterface::default(),
    );
    let mut state = session.state.lock().unwrap();
    let reference = |blob_id: String, kind| viewport_protocol::RuntimeBlobReference {
        blob_id,
        payload_kind: kind,
        payload_version: 1,
        byte_size: 8,
    };
    let manifest = viewport_protocol::AuthorizedRuntimeManifest {
        revision: "working-7".to_owned(),
        profile: viewport_protocol::RuntimeProfile::NativeMedium,
        hierarchy: reference(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            viewport_protocol::RuntimePayloadKind::Hierarchy,
        ),
        meshes: (0..200)
            .map(|index| {
                reference(
                    format!("{index:064x}"),
                    viewport_protocol::RuntimePayloadKind::Mesh,
                )
            })
            .collect(),
        materials: Vec::new(),
        textures: Vec::new(),
        redacted_blob_count: 0,
    };

    queue_server_event_for_request(
        &mut state,
        Some("runtime-manifest".to_owned()),
        ServerEvent::Session(SessionEvent::RuntimeManifest { manifest }),
    );

    assert!(state.pending_server_events.len() > 1);
    let expected_chunk_count = state.pending_server_events.len() as u32;
    for (expected_sequence, envelope) in state.pending_server_events.iter().enumerate() {
        assert_eq!(envelope.sequence, expected_sequence as u64 + 1);
        assert!(encoded_size(envelope).unwrap() <= MAX_APPLICATION_MESSAGE_BYTES);
        let ServerEvent::Session(SessionEvent::RuntimeManifestChunk {
            manifest_id,
            chunk_index,
            chunk_count,
            manifest,
        }) = &envelope.event
        else {
            panic!("oversized runtime manifests must be chunked");
        };
        assert_eq!(manifest_id, "runtime-manifest");
        assert_eq!(*chunk_index, expected_sequence as u32);
        assert_eq!(*chunk_count, expected_chunk_count);
        assert!(manifest.hierarchy.blob_id.starts_with('a'));
    }
}
