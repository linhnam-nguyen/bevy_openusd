use bevy::prelude::*;
use viewport_protocol::{ViewportEvent, ViewportEventEnvelope};

use super::bim_search;
use super::state::SceneSearchRequests;
use crate::viewport::api::scene_query::{SceneQueryService, SearchMatches, SearchResult};
use crate::viewport::api::{BimProvenanceService, ViewportEventOutbox};
use crate::viewport::diagnostics::performance::RendererCounters;
use crate::viewport::session::StageInfo;

/// Drains hierarchy-search-worker responses and publishes search results.
pub(crate) fn publish_scene_query_results(
    scene_query: Res<SceneQueryService>,
    bim_provenance: Option<Res<BimProvenanceService>>,
    stage_info: Option<Res<StageInfo>>,
    mut search_requests: ResMut<SceneSearchRequests>,
    mut outbox: ResMut<ViewportEventOutbox>,
    mut counters: Option<ResMut<RendererCounters>>,
) {
    let activation_generation = stage_info
        .as_ref()
        .map_or(0, |info| info.activation_generation);
    if let Some(bim_provenance) = bim_provenance {
        for result in bim_provenance.drain_results() {
            if result.activation_generation != activation_generation {
                continue;
            }
            let request_id = result.request_id;
            let event = match result.result {
                Ok(provenance) => ViewportEvent::BimPropertyProvenanceRead { provenance },
                Err(reason) => ViewportEvent::CommandRejected {
                    request_id: request_id.clone(),
                    reason,
                },
            };
            outbox.push(ViewportEventEnvelope::new(Some(request_id), event));
        }
    }

    for result in scene_query.drain_results() {
        let request_id = match &result {
            SearchResult::Hierarchy { request_id, .. } | SearchResult::Bim { request_id, .. } => {
                request_id
            }
        };
        let Some(request) = search_requests.pending.remove(request_id) else {
            // The read model will reject a response whose request is no
            // longer current; dropping it here also bounds pending metadata.
            continue;
        };
        let result_generation = match &result {
            SearchResult::Hierarchy {
                activation_generation,
                ..
            }
            | SearchResult::Bim {
                activation_generation,
                ..
            } => *activation_generation,
        };
        if result_generation != activation_generation {
            continue;
        }
        if let Some(ref mut counters) = counters {
            counters.query_results += 1;
            counters.record_query_latency_ms(request.submitted_at.elapsed().as_secs_f64() * 1000.0);
        }
        match result {
            SearchResult::Hierarchy {
                request_id,
                query,
                offset,
                total,
                source,
                matches,
                has_more,
                ..
            } => {
                let event = match matches {
                    SearchMatches::Scene(matches) => ViewportEvent::SearchResults {
                        query,
                        offset,
                        total,
                        matches: matches
                            .into_iter()
                            .filter_map(|result| result.into_scene_search_match())
                            .collect(),
                        has_more,
                    },
                    SearchMatches::Generic(matches) => ViewportEvent::HierarchySearchResults {
                        source,
                        query,
                        offset,
                        total,
                        matches,
                        has_more,
                    },
                };
                outbox.push(ViewportEventEnvelope::new(Some(request_id), event));
            }
            SearchResult::Bim {
                request_id, result, ..
            } => bim_search::publish_result(request_id, result, &mut outbox, &mut counters),
        }
    }
}
