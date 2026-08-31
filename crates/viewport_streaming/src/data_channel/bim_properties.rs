use log::warn;
use viewport_protocol::{
    BimPropertiesDeliveryError, BimPropertiesDeliveryErrorKind, BimPropertiesPage,
    BimPropertiesReadModel, BimPropertyGroupReadModel, BimPropertyReadModel, ServerEvent,
    ViewportEvent,
};

use super::constants::MAX_APPLICATION_MESSAGE_BYTES;
use super::dispatch::{encoded_size, next_server_envelope};
use super::events::queue_bounded_event;
use super::session::ApplicationSessionState;

const PAGE_COUNT_BOUND: u32 = viewport_protocol::MAX_BIM_PROPERTY_PAGES;

#[path = "bim_property_packer.rs"]
mod packer;
use packer::{PageContext, PageSizeEstimator, PropertyItem, encoded_empty_group_size};

pub(super) fn queue_bim_properties(
    state: &mut ApplicationSessionState,
    request_id: Option<String>,
    properties: BimPropertiesReadModel,
    diff: Option<viewport_protocol::BimPropertyDiffReadModel>,
) {
    let event = ServerEvent::Viewport(ViewportEvent::BimPropertiesRead {
        properties: properties.clone(),
        diff: diff.clone(),
    });
    if queue_bounded_event(state, request_id.as_deref(), event) {
        return;
    }

    let total_properties = properties
        .groups
        .iter()
        .map(|group| group.properties.len())
        .sum::<usize>();
    if total_properties > viewport_protocol::MAX_BIM_PROPERTY_COUNT {
        queue_error(
            state,
            request_id.as_deref(),
            properties.selection_revision,
            BimPropertiesDeliveryErrorKind::TooManyProperties,
            None,
            total_properties,
        );
        return;
    }

    let items = properties
        .groups
        .into_iter()
        .flat_map(|group| {
            let BimPropertyGroupReadModel {
                id,
                name,
                editable_group,
                properties,
            } = group;
            let encoded_empty_group_size = encoded_empty_group_size(id, &name, editable_group);
            properties.into_iter().map(move |property| PropertyItem {
                group_id: id,
                group_name: name.clone(),
                editable_group,
                encoded_size: encoded_property_size(&property),
                encoded_empty_group_size,
                property,
            })
        })
        .collect::<Vec<_>>();
    let page_context = PageContext {
        selection_revision: properties.selection_revision,
        total_properties: total_properties as u32,
        targets: &properties.targets,
        diff: diff.as_ref(),
        request_id: request_id.as_deref(),
    };

    let mut pages = Vec::new();
    let mut current = Vec::new();
    let mut estimator = PageSizeEstimator::new(state, &page_context, true);
    for item in items {
        if estimator.fits(&item) {
            estimator.add(&item);
            current.push(item);
            continue;
        }

        if current.is_empty() {
            let fits_with_metadata = page_fits(
                state,
                request_id.as_deref(),
                &make_page(
                    properties.selection_revision,
                    PAGE_COUNT_BOUND - 1,
                    PAGE_COUNT_BOUND,
                    total_properties as u32,
                    Some(properties.targets.as_slice()),
                    std::slice::from_ref(&item),
                    diff.as_ref(),
                ),
            );
            let fits_without_metadata = page_fits(
                state,
                request_id.as_deref(),
                &make_page(
                    properties.selection_revision,
                    PAGE_COUNT_BOUND - 1,
                    PAGE_COUNT_BOUND,
                    total_properties as u32,
                    None,
                    std::slice::from_ref(&item),
                    None,
                ),
            );
            let kind = if fits_without_metadata {
                BimPropertiesDeliveryErrorKind::OversizedMetadata
            } else {
                BimPropertiesDeliveryErrorKind::OversizedPropertyValue
            };
            if fits_with_metadata {
                estimator.add(&item);
                current.push(item);
                continue;
            }
            queue_error(
                state,
                request_id.as_deref(),
                properties.selection_revision,
                kind,
                Some(item.property.key.clone()),
                item.encoded_size,
            );
            return;
        }

        if !push_page(&mut pages, &page_context, &current, state) {
            queue_error(
                state,
                request_id.as_deref(),
                properties.selection_revision,
                BimPropertiesDeliveryErrorKind::TooManyProperties,
                None,
                pages.len(),
            );
            return;
        }
        current.clear();
        estimator = PageSizeEstimator::new(state, &page_context, false);
        if !estimator.fits(&item) {
            let fits_without_metadata = page_fits(
                state,
                request_id.as_deref(),
                &make_page(
                    properties.selection_revision,
                    PAGE_COUNT_BOUND - 1,
                    PAGE_COUNT_BOUND,
                    total_properties as u32,
                    None,
                    std::slice::from_ref(&item),
                    None,
                ),
            );
            if fits_without_metadata {
                estimator.add(&item);
                current.push(item);
                continue;
            }
            queue_error(
                state,
                request_id.as_deref(),
                properties.selection_revision,
                BimPropertiesDeliveryErrorKind::OversizedPropertyValue,
                Some(item.property.key.clone()),
                item.encoded_size,
            );
            return;
        }
        estimator.add(&item);
        current.push(item);
    }

    if !current.is_empty() && !push_page(&mut pages, &page_context, &current, state) {
        queue_error(
            state,
            request_id.as_deref(),
            properties.selection_revision,
            BimPropertiesDeliveryErrorKind::TooManyProperties,
            None,
            pages.len(),
        );
        return;
    }

    if pages.is_empty() && !push_page(&mut pages, &page_context, &[], state) {
        queue_error(
            state,
            request_id.as_deref(),
            properties.selection_revision,
            BimPropertiesDeliveryErrorKind::OversizedMetadata,
            None,
            properties.targets.len(),
        );
        return;
    }

    let page_count = pages.len() as u32;
    for (page_index, page) in pages.iter_mut().enumerate() {
        page.page_index = page_index as u32;
        page.page_count = page_count;
    }
    let envelopes = pages
        .into_iter()
        .map(|page| {
            next_server_envelope(
                state,
                request_id.as_deref(),
                ServerEvent::Viewport(ViewportEvent::BimPropertiesPage { page }),
            )
        })
        .collect::<Vec<_>>();
    state.pending_server_events.extend(envelopes);
}

fn push_page(
    pages: &mut Vec<BimPropertiesPage>,
    context: &PageContext<'_>,
    items: &[PropertyItem],
    state: &mut ApplicationSessionState,
) -> bool {
    if pages.len() >= PAGE_COUNT_BOUND as usize {
        return false;
    }
    let first_page = pages.is_empty();
    let page = make_page(
        context.selection_revision,
        PAGE_COUNT_BOUND - 1,
        PAGE_COUNT_BOUND,
        context.total_properties,
        first_page.then_some(context.targets),
        items,
        first_page.then_some(context.diff).flatten(),
    );
    if !page_fits(state, context.request_id, &page) {
        return false;
    }
    pages.push(page);
    true
}

fn make_page(
    selection_revision: u64,
    page_index: u32,
    page_count: u32,
    total_properties: u32,
    targets: Option<&[viewport_protocol::SceneAnchor]>,
    items: &[PropertyItem],
    diff: Option<&viewport_protocol::BimPropertyDiffReadModel>,
) -> BimPropertiesPage {
    let mut groups = Vec::<BimPropertyGroupReadModel>::new();
    for item in items {
        if let Some(group) = groups.last_mut()
            && group.id == item.group_id
        {
            group.properties.push(item.property.clone());
        } else {
            groups.push(BimPropertyGroupReadModel {
                id: item.group_id,
                name: item.group_name.clone(),
                editable_group: item.editable_group,
                properties: vec![item.property.clone()],
            });
        }
    }
    BimPropertiesPage {
        selection_revision,
        page_index,
        page_count,
        total_properties,
        targets: targets.map_or_else(Vec::new, ToOwned::to_owned),
        groups,
        diff: diff.cloned(),
    }
}

fn page_fits(
    state: &mut ApplicationSessionState,
    request_id: Option<&str>,
    page: &BimPropertiesPage,
) -> bool {
    let envelope = next_server_envelope(
        state,
        request_id,
        ServerEvent::Viewport(ViewportEvent::BimPropertiesPage { page: page.clone() }),
    );
    let fits = encoded_size(&envelope).is_some_and(|size| size <= MAX_APPLICATION_MESSAGE_BYTES);
    state.server_sequence = state.server_sequence.saturating_sub(1);
    fits
}

fn encoded_property_size(property: &BimPropertyReadModel) -> usize {
    serde_json::to_vec(property).map_or(usize::MAX, |bytes| bytes.len())
}

fn queue_error(
    state: &mut ApplicationSessionState,
    request_id: Option<&str>,
    selection_revision: u64,
    kind: BimPropertiesDeliveryErrorKind,
    property: Option<String>,
    encoded_bytes: usize,
) {
    let error = BimPropertiesDeliveryError {
        selection_revision,
        kind,
        property,
        encoded_bytes: encoded_bytes.min(u32::MAX as usize) as u32,
        max_bytes: MAX_APPLICATION_MESSAGE_BYTES.min(u32::MAX as usize) as u32,
    };
    if !queue_bounded_event(
        state,
        request_id,
        ServerEvent::Viewport(ViewportEvent::BimPropertiesError { error }),
    ) {
        warn!("[viewport-data-channel] BIM property delivery error could not be queued");
    }
}

#[cfg(test)]
#[path = "bim_properties_packing_tests.rs"]
mod packing_tests;
