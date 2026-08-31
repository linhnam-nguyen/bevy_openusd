use log::warn;
use viewport_protocol::{
    BimClassificationFieldCatalogue, BimClassificationFieldCataloguePage, ServerEvent,
    ViewportEvent,
};

use super::constants::MAX_APPLICATION_MESSAGE_BYTES;
use super::dispatch::{encoded_size, next_server_envelope};
use super::events::queue_bounded_event;
use super::session::ApplicationSessionState;

const PAGE_COUNT_BOUND: u32 = viewport_protocol::MAX_BIM_CLASSIFICATION_PAGES;
const PAGE_SIZE_ESTIMATE_MARGIN_BYTES: usize = 128;

pub(super) fn queue_bim_classification_field_catalogue(
    state: &mut ApplicationSessionState,
    request_id: Option<String>,
    catalogue: BimClassificationFieldCatalogue,
) {
    let event = ServerEvent::Viewport(ViewportEvent::BimClassificationFieldCatalogueChanged {
        catalogue: catalogue.clone(),
    });
    if queue_bounded_event(state, request_id.as_deref(), event) {
        return;
    }

    let total_fields = catalogue.fields.len() as u32;
    let base_size = encoded_empty_page_size(state, request_id.as_deref(), &catalogue);
    let mut pages = Vec::<Vec<viewport_protocol::BimClassificationFieldDescriptor>>::new();
    let mut current = Vec::new();
    let mut estimated_size = base_size;

    for field in catalogue.fields {
        let field_size = serde_json::to_vec(&field).map_or(usize::MAX, |bytes| bytes.len());
        let separator = usize::from(!current.is_empty());
        let additional = field_size.saturating_add(separator);
        if !current.is_empty()
            && estimated_size
                .saturating_add(additional)
                .saturating_add(PAGE_SIZE_ESTIMATE_MARGIN_BYTES)
                > MAX_APPLICATION_MESSAGE_BYTES
        {
            pages.push(current);
            current = Vec::new();
            estimated_size = base_size;
        }
        if current.is_empty()
            && estimated_size
                .saturating_add(field_size)
                .saturating_add(PAGE_SIZE_ESTIMATE_MARGIN_BYTES)
                > MAX_APPLICATION_MESSAGE_BYTES
        {
            warn!(
                "[viewport-data-channel] dropping BIM classification descriptor because it exceeds the application message limit"
            );
            return;
        }
        estimated_size = estimated_size.saturating_add(field_size + separator);
        current.push(field);
    }
    if !current.is_empty() {
        pages.push(current);
    }
    if pages.is_empty() || pages.len() > PAGE_COUNT_BOUND as usize {
        warn!("[viewport-data-channel] BIM classification catalogue exceeds its page bound");
        return;
    }

    let page_count = pages.len() as u32;
    for (page_index, fields) in pages.into_iter().enumerate() {
        let page = BimClassificationFieldCataloguePage {
            semantic_revision: catalogue.semantic_revision,
            page_index: page_index as u32,
            page_count,
            total_fields,
            fields,
        };
        if !queue_bounded_event(
            state,
            request_id.as_deref(),
            ServerEvent::Viewport(ViewportEvent::BimClassificationFieldCataloguePage { page }),
        ) {
            warn!(
                "[viewport-data-channel] BIM classification catalogue page exceeded the application message limit"
            );
            return;
        }
    }
}

fn encoded_empty_page_size(
    state: &mut ApplicationSessionState,
    request_id: Option<&str>,
    catalogue: &BimClassificationFieldCatalogue,
) -> usize {
    let page = BimClassificationFieldCataloguePage {
        semantic_revision: catalogue.semantic_revision,
        page_index: PAGE_COUNT_BOUND - 1,
        page_count: PAGE_COUNT_BOUND,
        total_fields: catalogue.fields.len() as u32,
        fields: Vec::new(),
    };
    let envelope = next_server_envelope(
        state,
        request_id,
        ServerEvent::Viewport(ViewportEvent::BimClassificationFieldCataloguePage { page }),
    );
    let size = encoded_size(&envelope).unwrap_or(usize::MAX);
    state.server_sequence = state.server_sequence.saturating_sub(1);
    size
}
