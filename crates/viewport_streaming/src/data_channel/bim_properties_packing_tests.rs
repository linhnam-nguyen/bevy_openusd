use std::time::Instant;

use viewport_protocol::{ServerEvent, SessionId, ViewportEvent, ViewportReadModel};

use crate::application::RenderServerInterface;
use crate::data_channel::constants::MAX_APPLICATION_MESSAGE_BYTES;
use crate::data_channel::dispatch::encoded_size;
use crate::data_channel::events::queue_server_event_for_request;
use crate::data_channel::session::ApplicationSession;

#[test]
fn large_bim_property_response_packs_at_scale_without_prefix_rebuilds() {
    let property_count = 4_096;
    let session = ApplicationSession::new(
        SessionId::new("session-1"),
        ViewportReadModel::unloaded("stage.usda"),
        RenderServerInterface::default(),
    );
    let mut state = session.state.lock().unwrap();
    let properties = viewport_protocol::BimPropertiesReadModel {
        targets: vec![viewport_protocol::SceneAnchor::active_session(
            "/World/Window",
        )],
        selection_revision: 13,
        groups: vec![viewport_protocol::BimPropertyGroupReadModel {
            id: viewport_protocol::BimPropertyGroupId::Semantic,
            name: "Semantic".to_owned(),
            editable_group: true,
            properties: (0..property_count)
                .map(|index| viewport_protocol::BimPropertyReadModel {
                    key: format!("Property-{index}"),
                    label: format!("Property {index}"),
                    scope: viewport_protocol::BimPropertyScope::Other,
                    group_id: viewport_protocol::BimPropertyGroupId::Semantic,
                    value: viewport_protocol::CommonValue::Same(
                        viewport_protocol::CanonicalValue::Text("value".to_owned()),
                    ),
                    target_values: vec![viewport_protocol::CanonicalValue::Text(
                        "value".to_owned(),
                    )],
                    measurement: None,
                    units: Vec::new(),
                    current_display_unit: None,
                    editable: true,
                })
                .collect(),
        }],
    };

    let started = Instant::now();
    queue_server_event_for_request(
        &mut state,
        Some("bim-scale".to_owned()),
        ServerEvent::Viewport(ViewportEvent::BimPropertiesRead {
            properties,
            diff: None,
        }),
    );

    let mut received = 0;
    for envelope in &state.pending_server_events {
        assert_eq!(envelope.request_id.as_deref(), Some("bim-scale"));
        assert!(encoded_size(envelope).is_some_and(|size| size <= MAX_APPLICATION_MESSAGE_BYTES));
        let ServerEvent::Viewport(ViewportEvent::BimPropertiesPage { page }) = &envelope.event
        else {
            panic!("large BIM properties must remain paged instead of failing the response");
        };
        received += page
            .groups
            .iter()
            .map(|group| group.properties.len())
            .sum::<usize>();
    }
    assert_eq!(received, property_count);
    eprintln!(
        "M8-OR2-C1+ paging scale: properties={property_count} pages={} elapsed_ms={:.3}",
        state.pending_server_events.len(),
        started.elapsed().as_secs_f64() * 1_000.0,
    );
}
