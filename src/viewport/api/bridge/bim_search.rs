use bevy::prelude::*;
use viewport_protocol::{BimSearchQuery, BimSearchResult, ViewportEvent, ViewportEventEnvelope};

use super::helpers::reject;
use super::state::{SceneSearchRequest, SceneSearchRequests};
use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::api::scene_query::SceneQueryService;
use crate::viewport::diagnostics::performance::RendererCounters;
use crate::viewport::semantic::SemanticSyncState;

pub(super) fn publish_result(
    request_id: String,
    result: Result<BimSearchResult, String>,
    outbox: &mut ViewportEventOutbox,
    counters: &mut Option<ResMut<RendererCounters>>,
) {
    match result {
        Ok(result) => outbox.push(ViewportEventEnvelope::new(
            Some(request_id),
            ViewportEvent::BimSearchResults { result },
        )),
        Err(error) => {
            if let Some(counters) = counters {
                counters.query_failures += 1;
            }
            reject(outbox, request_id, error);
        }
    }
}

pub(super) fn dispatch(
    request_id: String,
    query: BimSearchQuery,
    semantic: Option<&SemanticSyncState>,
    scene_query: &SceneQueryService,
    search_requests: &mut SceneSearchRequests,
    outbox: &mut ViewportEventOutbox,
    counters: &mut Option<ResMut<RendererCounters>>,
) {
    let Some(snapshot) = semantic.and_then(SemanticSyncState::shared_snapshot) else {
        reject(
            outbox,
            request_id,
            "BIM search semantic snapshot is unavailable".to_owned(),
        );
        return;
    };
    if scene_query.submit_bim_search(request_id.clone(), query, snapshot) {
        search_requests.pending.clear();
        search_requests.pending.insert(
            request_id,
            SceneSearchRequest {
                query: "bim".to_owned(),
                offset: 0,
                submitted_at: std::time::Instant::now(),
            },
        );
        if let Some(counters) = counters {
            counters.query_requests += 1;
        }
    } else {
        if let Some(counters) = counters {
            counters.query_failures += 1;
        }
        reject(
            outbox,
            request_id,
            "BIM search worker is unavailable".to_owned(),
        );
    }
}
