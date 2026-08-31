use viewport_protocol::{
    BimClassificationFieldCatalogue, BimFieldKey, BimPropertyScope, ServerEvent, SessionId,
    ViewportEvent, ViewportReadModel,
};

use crate::application::RenderServerInterface;
use crate::data_channel::constants::MAX_APPLICATION_MESSAGE_BYTES;
use crate::data_channel::dispatch::encoded_size;
use crate::data_channel::events::queue_server_event_for_request;
use crate::data_channel::session::ApplicationSession;

#[test]
fn oversized_bim_classification_catalogue_is_paged_and_reconstructs_exactly() {
    let session = ApplicationSession::new(
        SessionId::new("session-1"),
        ViewportReadModel::unloaded("stage.usda"),
        RenderServerInterface::default(),
    );
    let mut state = session.state.lock().unwrap();
    let catalogue = BimClassificationFieldCatalogue {
        semantic_revision: 41,
        fields: (0..4096)
            .map(|index| {
                viewport_protocol::BimClassificationFieldDescriptor::new(
                    BimFieldKey::property(format!("BIM:Instance:Property-{index}")),
                    format!("Property {index}"),
                    BimPropertyScope::Instance,
                )
            })
            .collect(),
    };
    let expected = catalogue.fields.clone();

    queue_server_event_for_request(
        &mut state,
        Some("bim-catalogue".to_owned()),
        ServerEvent::Viewport(ViewportEvent::BimClassificationFieldCatalogueChanged { catalogue }),
    );

    assert!(state.pending_server_events.len() > 1);
    let page_count = state.pending_server_events.len() as u32;
    let mut reconstructed = Vec::new();
    for (index, envelope) in state.pending_server_events.iter().enumerate() {
        assert_eq!(envelope.sequence, index as u64 + 1);
        assert_eq!(envelope.request_id.as_deref(), Some("bim-catalogue"));
        assert!(encoded_size(envelope).unwrap() <= MAX_APPLICATION_MESSAGE_BYTES);
        let ServerEvent::Viewport(ViewportEvent::BimClassificationFieldCataloguePage { page }) =
            &envelope.event
        else {
            panic!("oversized BIM catalogue must be sent as catalogue pages");
        };
        assert_eq!(page.semantic_revision, 41);
        assert_eq!(page.page_index, index as u32);
        assert_eq!(page.page_count, page_count);
        assert_eq!(page.total_fields, 4096);
        reconstructed.extend(page.fields.clone());
    }
    assert_eq!(reconstructed, expected);
}
