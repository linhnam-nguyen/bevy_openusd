use viewport_protocol::{BimPropertyGroupId, BimPropertyReadModel, ServerEvent, ViewportEvent};

use crate::data_channel::constants::MAX_APPLICATION_MESSAGE_BYTES;
use crate::data_channel::dispatch::{encoded_size, next_server_envelope};
use crate::data_channel::session::ApplicationSessionState;

const PAGE_COUNT_BOUND: u32 = viewport_protocol::MAX_BIM_PROPERTY_PAGES;
const PAGE_SIZE_ESTIMATE_MARGIN_BYTES: usize = 64;

pub(super) struct PropertyItem {
    pub(super) group_id: BimPropertyGroupId,
    pub(super) group_name: String,
    pub(super) editable_group: bool,
    pub(super) property: BimPropertyReadModel,
    pub(super) encoded_size: usize,
    pub(super) encoded_empty_group_size: usize,
}

pub(super) struct PageContext<'a> {
    pub(super) selection_revision: u64,
    pub(super) total_properties: u32,
    pub(super) targets: &'a [viewport_protocol::SceneAnchor],
    pub(super) diff: Option<&'a viewport_protocol::BimPropertyDiffReadModel>,
    pub(super) request_id: Option<&'a str>,
}

/// Conservative linear page-size accounting used while selecting page items.
/// The final page is still checked against the authoritative envelope after
/// it is built, but candidate prefixes are never cloned or serialized.
pub(super) struct PageSizeEstimator {
    encoded_bytes: usize,
    last_group_id: Option<BimPropertyGroupId>,
}

impl PageSizeEstimator {
    pub(super) fn new(
        state: &mut ApplicationSessionState,
        context: &PageContext<'_>,
        include_metadata: bool,
    ) -> Self {
        let page = super::make_page(
            context.selection_revision,
            PAGE_COUNT_BOUND - 1,
            PAGE_COUNT_BOUND,
            context.total_properties,
            include_metadata.then_some(context.targets),
            &[],
            include_metadata.then_some(context.diff).flatten(),
        );
        let envelope = next_server_envelope(
            state,
            context.request_id,
            ServerEvent::Viewport(ViewportEvent::BimPropertiesPage { page }),
        );
        let encoded_bytes = encoded_size(&envelope)
            .unwrap_or(usize::MAX)
            .saturating_add(PAGE_SIZE_ESTIMATE_MARGIN_BYTES);
        state.server_sequence = state.server_sequence.saturating_sub(1);
        Self {
            encoded_bytes,
            last_group_id: None,
        }
    }

    pub(super) fn fits(&self, item: &PropertyItem) -> bool {
        self.encoded_bytes
            .checked_add(self.additional_bytes(item))
            .is_some_and(|size| size <= MAX_APPLICATION_MESSAGE_BYTES)
    }

    pub(super) fn add(&mut self, item: &PropertyItem) {
        self.encoded_bytes = self
            .encoded_bytes
            .saturating_add(self.additional_bytes(item));
        self.last_group_id = Some(item.group_id);
    }

    fn additional_bytes(&self, item: &PropertyItem) -> usize {
        let separator = usize::from(self.last_group_id.is_some());
        let group_prefix = if self.last_group_id == Some(item.group_id) {
            1
        } else {
            item.encoded_empty_group_size.saturating_add(separator)
        };
        group_prefix.saturating_add(item.encoded_size)
    }
}

pub(super) fn encoded_empty_group_size(
    id: BimPropertyGroupId,
    name: &str,
    editable_group: bool,
) -> usize {
    serde_json::to_vec(&viewport_protocol::BimPropertyGroupReadModel {
        id,
        name: name.to_owned(),
        editable_group,
        properties: Vec::new(),
    })
    .map_or(usize::MAX, |bytes| bytes.len())
}
