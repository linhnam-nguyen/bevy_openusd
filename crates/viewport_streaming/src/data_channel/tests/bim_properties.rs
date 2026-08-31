use viewport_protocol::{ServerEvent, SessionId, ViewportEvent, ViewportReadModel};

use crate::application::RenderServerInterface;
use crate::data_channel::constants::MAX_APPLICATION_MESSAGE_BYTES;
use crate::data_channel::dispatch::encoded_size;
use crate::data_channel::events::queue_server_event_for_request;
use crate::data_channel::session::ApplicationSession;

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
