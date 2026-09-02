use log::warn;
use viewport_protocol::{ServerEvent, SessionEvent, ViewportEvent, ViewportReadModel};

use super::bim_classification::queue_bim_classification_field_catalogue;
use super::bim_properties::queue_bim_properties;
use super::chunks::{queue_runtime_blob, queue_runtime_manifest, queue_snapshot};
use super::constants::MAX_APPLICATION_MESSAGE_BYTES;
use super::dispatch::{encoded_size, next_server_envelope};
use super::session::ApplicationSessionState;

pub(super) fn queue_server_event_for_request(
    state: &mut ApplicationSessionState,
    request_id: Option<String>,
    event: ServerEvent,
) {
    match event {
        ServerEvent::Session(SessionEvent::Snapshot { state: snapshot }) => {
            queue_snapshot(state, request_id, snapshot, true);
        }
        ServerEvent::Viewport(ViewportEvent::Snapshot { state: snapshot }) => {
            queue_snapshot(state, request_id, *snapshot, false);
        }
        ServerEvent::Viewport(ViewportEvent::SearchResults {
            query,
            offset,
            total,
            matches,
            has_more,
        }) => {
            queue_search_results(state, request_id, query, offset, total, matches, has_more);
        }
        ServerEvent::Viewport(ViewportEvent::SceneChildren { page }) => {
            queue_scene_children_page(state, request_id, page);
        }
        ServerEvent::Viewport(ViewportEvent::BimPropertiesRead { properties, diff }) => {
            queue_bim_properties(state, request_id, properties, diff);
        }
        ServerEvent::Viewport(ViewportEvent::BimClassificationFieldCatalogueChanged {
            catalogue,
        }) => {
            queue_bim_classification_field_catalogue(state, request_id, catalogue);
        }
        ServerEvent::Session(SessionEvent::RuntimeManifest { manifest }) => {
            queue_runtime_manifest(state, request_id.as_deref(), manifest);
        }
        ServerEvent::Session(SessionEvent::RuntimeBlobChunk {
            blob_id,
            chunk_index: _,
            chunk_count: _,
            bytes,
        }) => {
            queue_runtime_blob(state, request_id.as_deref(), blob_id, bytes);
        }
        event => {
            if !queue_bounded_event(state, request_id.as_deref(), event) {
                warn!(
                    "[viewport-data-channel] dropping oversized application event instead of blocking the queue"
                );
            }
        }
    }
}

pub(super) fn queue_search_results(
    state: &mut ApplicationSessionState,
    request_id: Option<String>,
    query: String,
    offset: u32,
    total: u32,
    matches: Vec<viewport_protocol::SceneSearchMatch>,
    has_more: bool,
) {
    let event = ServerEvent::Viewport(ViewportEvent::SearchResults {
        query: query.clone(),
        offset,
        total,
        matches: matches.clone(),
        has_more,
    });
    if queue_bounded_event(state, request_id.as_deref(), event) {
        return;
    }

    if matches.len() > 1 {
        let split = matches.len() / 2;
        let mut tail = matches;
        let head = tail.split_off(split);
        queue_search_results(
            state,
            request_id.clone(),
            query.clone(),
            offset,
            total,
            tail,
            true,
        );
        queue_search_results(
            state,
            request_id,
            query,
            offset.saturating_add(split as u32),
            total,
            head,
            has_more,
        );
        return;
    }

    if let Some(mut result) = matches.into_iter().next() {
        result.reveal_pages.clear();
        let fallback = ServerEvent::Viewport(ViewportEvent::SearchResults {
            query,
            offset,
            total,
            matches: vec![result],
            has_more,
        });
        if queue_bounded_event(state, request_id.as_deref(), fallback) {
            return;
        }
    }

    warn!(
        "[viewport-data-channel] dropping search result page because one result exceeds the application message limit"
    );
}

pub(super) fn queue_scene_children_page(
    state: &mut ApplicationSessionState,
    request_id: Option<String>,
    page: viewport_protocol::SceneChildrenPage,
) {
    let event = ServerEvent::Viewport(ViewportEvent::SceneChildren { page: page.clone() });
    if queue_bounded_event(state, request_id.as_deref(), event) {
        return;
    }

    if page.nodes.len() > 1 {
        let split = page.nodes.len() / 2;
        let viewport_protocol::SceneChildrenPage {
            parent,
            page,
            page_size,
            total,
            nodes,
        } = page;
        let mut tail = nodes;
        let head = tail.split_off(split);
        let first = viewport_protocol::SceneChildrenPage {
            parent: parent.clone(),
            page,
            page_size,
            total,
            nodes: tail,
        };
        let second = viewport_protocol::SceneChildrenPage {
            parent,
            page,
            page_size,
            total,
            nodes: head,
        };
        queue_scene_children_page(state, request_id.clone(), first);
        queue_scene_children_page(state, request_id, second);
        return;
    }

    warn!(
        "[viewport-data-channel] dropping scene child node because it exceeds the application message limit"
    );
}

pub(super) fn queue_bounded_event(
    state: &mut ApplicationSessionState,
    request_id: Option<&str>,
    event: ServerEvent,
) -> bool {
    let envelope = next_server_envelope(state, request_id, event);
    if encoded_size(&envelope).is_some_and(|size| size <= MAX_APPLICATION_MESSAGE_BYTES) {
        state.pending_server_events.push_back(envelope);
        true
    } else {
        state.server_sequence = state.server_sequence.saturating_sub(1);
        false
    }
}

pub(super) fn snapshot_event(snapshot: ViewportReadModel, session_snapshot: bool) -> ServerEvent {
    if session_snapshot {
        ServerEvent::Session(SessionEvent::Snapshot { state: snapshot })
    } else {
        ServerEvent::Viewport(ViewportEvent::Snapshot {
            state: Box::new(snapshot),
        })
    }
}
