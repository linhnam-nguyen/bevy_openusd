use std::time::Instant;

use bevy::prelude::*;
use viewport_protocol::{
    HierarchySource, PROTOCOL_VERSION, ViewportCommand, ViewportEvent, ViewportEventEnvelope,
};

use super::super::helpers::reject;
use super::super::state::{SceneSearchRequest, SceneSearchRequests};
use super::super::{bim_properties, bim_search};
use crate::viewport::api::scene_query::SceneQueryService;
use crate::viewport::api::{
    ActiveHierarchyProvider, BimProvenanceService, CurrentHierarchyProjection, SceneAnchorIndex,
    ViewportCommandInbox, ViewportEventOutbox, refresh_projection_visibility,
};
use crate::viewport::diagnostics::performance::RendererCounters;
use crate::viewport::scene::SelectedTargets;
use crate::viewport::session::{StageHandle, StageInfo};

/// Routes scene-query commands to the current hierarchy projection.
pub(crate) fn dispatch_scene_query_commands(
    mut inbox: ResMut<ViewportCommandInbox>,
    scene_index: Res<SceneAnchorIndex>,
    mut current_projection: ResMut<CurrentHierarchyProjection>,
    mut provider: Option<ResMut<ActiveHierarchyProvider>>,
    mut bim_classification: Option<ResMut<crate::viewport::api::BimClassificationRecipeState>>,
    semantic: Option<Res<crate::viewport::semantic::SemanticSyncState>>,
    semantic_diff: Option<Res<crate::viewport::semantic::SemanticDiffState>>,
    selection: Option<Res<SelectedTargets>>,
    stage_handle: Option<Res<StageHandle>>,
    stage_info: Option<Res<StageInfo>>,
    scene_query: Res<SceneQueryService>,
    bim_provenance: Option<Res<BimProvenanceService>>,
    mut search_requests: ResMut<SceneSearchRequests>,
    mut outbox: ResMut<ViewportEventOutbox>,
    mut counters: Option<ResMut<RendererCounters>>,
) {
    let activation_generation = stage_info
        .as_ref()
        .map_or(0, |info| info.activation_generation);
    for envelope in inbox.take_scene_query_commands() {
        let request_id = envelope.request_id;
        if envelope.protocol_version != PROTOCOL_VERSION {
            reject(
                &mut outbox,
                request_id,
                format!(
                    "unsupported protocol version {}; expected {}",
                    envelope.protocol_version, PROTOCOL_VERSION
                ),
            );
            continue;
        }

        match envelope.command {
            ViewportCommand::RequestSceneChildren {
                parent,
                page,
                page_size,
            } => outbox.push(ViewportEventEnvelope::new(
                Some(request_id),
                ViewportEvent::SceneChildren {
                    page: scene_index.children_page(parent.as_ref(), page, page_size),
                },
            )),
            ViewportCommand::RequestHierarchyChildren {
                source,
                parent_id,
                page,
                page_size,
            } => {
                if source != current_projection.source() {
                    reject(
                        &mut outbox,
                        request_id,
                        format!("hierarchy provider {source:?} is not active"),
                    );
                    continue;
                }
                match current_projection.children_page(parent_id.as_ref(), page, page_size) {
                    Ok(page) => outbox.push(ViewportEventEnvelope::new(
                        Some(request_id),
                        ViewportEvent::HierarchyChildren { source, page },
                    )),
                    Err(error) => reject(&mut outbox, request_id, error),
                }
            }
            ViewportCommand::SearchScene {
                query,
                offset,
                limit,
            } => {
                let query_text = query.clone();
                if current_projection.source() != HierarchySource::Prim {
                    reject(
                        &mut outbox,
                        request_id,
                        "prim scene search provider is not active".to_owned(),
                    );
                    continue;
                }
                if scene_query.submit_search(
                    request_id.clone(),
                    query,
                    offset,
                    limit,
                    current_projection.snapshot(),
                    HierarchySource::Prim,
                    false,
                    activation_generation,
                ) {
                    // Search is a single latest-query projection in the
                    // viewport read model. Dropping older metadata here also
                    // makes worker-side query coalescing safe: superseded
                    // responses are ignored when they arrive.
                    search_requests.pending.clear();
                    search_requests.pending.insert(
                        request_id,
                        SceneSearchRequest {
                            query: query_text,
                            offset,
                            submitted_at: Instant::now(),
                        },
                    );
                    if let Some(ref mut counters) = counters {
                        counters.query_requests += 1;
                    }
                } else {
                    if let Some(ref mut counters) = counters {
                        counters.query_failures += 1;
                    }
                    reject(
                        &mut outbox,
                        request_id,
                        "hierarchy search worker is unavailable".to_owned(),
                    );
                }
            }
            ViewportCommand::SearchHierarchy {
                source,
                query,
                offset,
                limit,
            } => {
                if source != current_projection.source() {
                    reject(
                        &mut outbox,
                        request_id,
                        format!("hierarchy provider {source:?} is not active"),
                    );
                    continue;
                }
                let query_text = query.clone();
                if scene_query.submit_search(
                    request_id.clone(),
                    query,
                    offset,
                    limit,
                    current_projection.snapshot(),
                    source,
                    true,
                    activation_generation,
                ) {
                    search_requests.pending.clear();
                    search_requests.pending.insert(
                        request_id,
                        SceneSearchRequest {
                            query: query_text,
                            offset,
                            submitted_at: Instant::now(),
                        },
                    );
                    if let Some(ref mut counters) = counters {
                        counters.query_requests += 1;
                    }
                } else {
                    if let Some(ref mut counters) = counters {
                        counters.query_failures += 1;
                    }
                    reject(
                        &mut outbox,
                        request_id,
                        "hierarchy search worker is unavailable".to_owned(),
                    );
                }
            }
            ViewportCommand::SearchBim { query } => {
                bim_search::dispatch(
                    request_id,
                    query,
                    semantic.as_ref().map(|state| &**state),
                    activation_generation,
                    &scene_query,
                    &mut search_requests,
                    &mut outbox,
                    &mut counters,
                );
            }
            ViewportCommand::RequestBimProperties => {
                bim_properties::dispatch(
                    request_id,
                    selection.as_deref(),
                    semantic.as_deref(),
                    semantic_diff.as_deref(),
                    &mut outbox,
                );
            }
            ViewportCommand::RequestBimPropertyProvenance {
                target,
                property,
                history_head,
            } => {
                let Some(stage_handle) = stage_handle.as_deref() else {
                    bim_properties::emit_unavailable(
                        request_id,
                        target,
                        property,
                        history_head,
                        &mut outbox,
                    );
                    continue;
                };
                let Some(bim_provenance) = bim_provenance.as_deref() else {
                    bim_properties::emit_unavailable(
                        request_id,
                        target,
                        property,
                        history_head,
                        &mut outbox,
                    );
                    continue;
                };
                bim_properties::submit_provenance(
                    request_id,
                    target,
                    property,
                    history_head,
                    semantic.as_deref(),
                    semantic_diff.as_deref(),
                    &stage_handle.path,
                    activation_generation,
                    &bim_provenance,
                    &mut outbox,
                );
            }
            ViewportCommand::SetHierarchySource {
                source,
                classification_recipe,
            } => {
                let Some(provider) = provider.as_deref_mut() else {
                    reject(
                        &mut outbox,
                        request_id,
                        "hierarchy provider state is unavailable".to_owned(),
                    );
                    continue;
                };
                let projection = match source {
                    HierarchySource::Prim => Ok(scene_index.prim_projection()),
                    HierarchySource::BimClassification => match (
                        classification_recipe.as_ref(),
                        semantic.as_ref().and_then(|state| state.snapshot()),
                        semantic.as_ref().and_then(|state| state.shared_bim_index()),
                    ) {
                        (Some(recipe), Some(snapshot), Some(index)) => {
                            crate::viewport::bim::BimReadService::with_index(snapshot, index)
                                .classification_projection(recipe)
                                .map_err(|error| error.to_string())
                        }
                        _ => Err(
                            "BIM classification recipe or semantic snapshot is unavailable"
                                .to_owned(),
                        ),
                    },
                };
                match projection {
                    Ok(projection) => {
                        let mut projection = projection;
                        refresh_projection_visibility(&mut projection, &scene_index);
                        *current_projection = projection;
                        provider.set(source, classification_recipe);
                    }
                    Err(error) => reject(&mut outbox, request_id, error),
                }
            }
            ViewportCommand::SetBimClassificationRecipe { recipe } => {
                if let Some(bim_classification) = bim_classification.as_deref_mut() {
                    bim_classification.set(recipe);
                } else {
                    reject(
                        &mut outbox,
                        request_id,
                        "BIM classification presentation is unavailable".to_owned(),
                    );
                }
            }
            _ => unreachable!("scene query inbox only contains query commands"),
        }
    }
}
