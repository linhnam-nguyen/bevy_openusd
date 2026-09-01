mod action;
mod changed_info;
mod classification_catalogue;
mod delivery;
mod delivery_worker;
mod subtree;

use std::{sync::Arc, time::Instant};

pub(crate) use action::SubtreeUpdateError;
pub(in crate::viewport::semantic) use action::{SemanticExtractionOutcome, SemanticSyncAction};
pub(in crate::viewport::semantic) use changed_info::changed_info_update;
pub(in crate::viewport::semantic) use delivery::{
    attach_render_blobs_to_action, queue_runtime_delivery,
};
pub(crate) use delivery::{drain_runtime_delivery_results, flush_pending_runtime_delivery};
pub(crate) use delivery_worker::RuntimeDeliveryRuntime;
pub(in crate::viewport::semantic) use subtree::resync_subtree_update;

use bevy::prelude::World;
use usd_bevy::{LiveRevision, LiveStage, PendingStageChanges};
use usd_model::SnapshotSource;
use usd_semantic::{SemanticConfig, SemanticExtractor};

use super::diff::SemanticDiffState;
use super::state::SemanticSyncState;
use super::worker::SemanticWorkingStore;
use crate::viewport::bim::BimReadService;

/// Synchronize the semantic working store from the retained live-stage batch.
///
/// This is an exclusive system because `LiveStage` is a non-send resource and
/// extraction must borrow its OpenUSD stage while the resulting ECS resource
/// state is updated. It runs after `LiveStagePlugin` has drained the batch.
pub(crate) fn synchronize_live_stage(world: &mut World) {
    let started = Instant::now();
    synchronize_live_stage_inner(world);
    if let Some(mut counters) =
        world.get_resource_mut::<crate::viewport::diagnostics::performance::RendererCounters>()
    {
        counters.total_semantic_postupdate_ms += started.elapsed().as_secs_f64() * 1000.0;
    }
}

fn synchronize_live_stage_inner(world: &mut World) {
    if world.get_non_send::<LiveStage>().is_some()
        && let Some(mut c) =
            world.get_resource_mut::<crate::viewport::diagnostics::performance::RendererCounters>()
    {
        c.semantic_sync_calls += 1;
    }

    let info = (|| {
        let live = world.get_non_send::<LiveStage>()?;
        let pending_batch = world.resource::<PendingStageChanges>().batch().cloned();
        let state = world.resource::<SemanticSyncState>();
        Some((
            live.session_id(),
            live.current_revision(),
            pending_batch,
            state.snapshot.is_some(),
            state.session_id,
            state.revision,
        ))
    })();

    let Some((
        session_id,
        live_revision,
        pending_batch,
        has_snapshot,
        previous_session,
        previous_revision,
    )) = info
    else {
        return;
    };

    let same_session = previous_session == Some(session_id);
    if same_session && has_snapshot && pending_batch.is_none() {
        if let Some(mut c) =
            world.get_resource_mut::<crate::viewport::diagnostics::performance::RendererCounters>()
        {
            c.semantic_idle_skips += 1;
        }
        return;
    }

    if same_session
        && pending_batch
            .as_ref()
            .is_some_and(|batch| batch.revision <= previous_revision.unwrap_or_default())
    {
        return;
    }

    let previous_snapshot = if same_session {
        world.resource::<SemanticSyncState>().shared_snapshot()
    } else {
        None
    };

    let root_count = pending_batch
        .as_ref()
        .map(|b| {
            if b.has_resync() {
                b.resync_roots().len()
            } else {
                0
            }
        })
        .unwrap_or(0);

    if previous_snapshot.is_none() {
        if let Some(mut c) =
            world.get_resource_mut::<crate::viewport::diagnostics::performance::RendererCounters>()
        {
            c.semantic_initial_extractions += 1;
        }
    } else if pending_batch.is_none() {
        if let Some(mut c) =
            world.get_resource_mut::<crate::viewport::diagnostics::performance::RendererCounters>()
        {
            c.semantic_idle_skips += 1;
        }
        return;
    }

    let extractor = SemanticExtractor::new(world.resource::<SemanticSyncState>().config());
    let source = SnapshotSource::Working {
        session: "viewport-working".to_owned(),
        live_revision: live_revision.0,
    };

    let extraction_started = Instant::now();
    let (update, outcome) = {
        let live = world
            .get_non_send::<LiveStage>()
            .expect("live stage exists");
        match previous_snapshot {
            None => match extractor.extract(&live.stage, source) {
                Ok(snapshot) => (
                    Some(SemanticSyncAction::Replace(Arc::new(snapshot))),
                    SemanticExtractionOutcome::Initial,
                ),
                Err(error) => {
                    bevy::log::error!("[semantic-sync] initial snapshot failed: {error:#}");
                    (None, SemanticExtractionOutcome::InitialFailure)
                }
            },
            Some(previous_snapshot) => {
                let Some(batch) = pending_batch else {
                    return;
                };
                let previous_revision = previous_revision.unwrap_or(LiveRevision::default());
                if batch.revision <= previous_revision {
                    return;
                }
                if batch.has_resync() {
                    let all_resynced: Vec<&str> = batch
                        .changes
                        .iter()
                        .flat_map(|c| c.resynced.iter().map(String::as_str))
                        .collect();
                    let mut unnormalizable = false;
                    for r in &all_resynced {
                        if let Err(err) = usd_bevy::validate_prim_path(r) {
                            bevy::log::warn!(
                                target: "semantic_sync",
                                resync_fallback_reason = "unnormalizable_root",
                                root_count = all_resynced.len(),
                                live_revision = live_revision.0,
                                "[semantic-sync] root '{r}' cannot represent a safe OpenUSD prim path: {err:#}; falling back to full snapshot rebuild"
                            );
                            unnormalizable = true;
                            break;
                        }
                    }

                    if unnormalizable {
                        match extractor.extract(&live.stage, source) {
                            Ok(snapshot) => (
                                Some(SemanticSyncAction::Replace(Arc::new(snapshot))),
                                SemanticExtractionOutcome::Fallback,
                            ),
                            Err(err) => {
                                bevy::log::error!(
                                    "[semantic-sync] full snapshot fallback failed: {err:#}"
                                );
                                (None, SemanticExtractionOutcome::Fallback)
                            }
                        }
                    } else {
                        let roots = batch.resync_roots();
                        if roots.contains(&"/".to_string()) || roots.is_empty() {
                            bevy::log::warn!(
                                target: "semantic_sync",
                                resync_fallback_reason = "root_is_stage_root_or_empty",
                                root_count = roots.len(),
                                live_revision = live_revision.0,
                                "[semantic-sync] stage root '/' or empty roots in batch; falling back to full snapshot rebuild"
                            );
                            match extractor.extract(&live.stage, source) {
                                Ok(snapshot) => (
                                    Some(SemanticSyncAction::Replace(Arc::new(snapshot))),
                                    SemanticExtractionOutcome::Fallback,
                                ),
                                Err(error) => {
                                    bevy::log::error!(
                                        "[semantic-sync] resync full rebuild failed: {error:#}"
                                    );
                                    (None, SemanticExtractionOutcome::Fallback)
                                }
                            }
                        } else {
                            match resync_subtree_update(
                                &live.stage,
                                &extractor,
                                Arc::clone(&previous_snapshot),
                                &batch,
                                source.clone(),
                            ) {
                                Ok(update) => (
                                    Some(SemanticSyncAction::Delta(update)),
                                    SemanticExtractionOutcome::Subtree,
                                ),
                                Err(err) => {
                                    let reason = err.fallback_reason();
                                    bevy::log::warn!(
                                        target: "semantic_sync",
                                        resync_fallback_reason = reason,
                                        root_count = roots.len(),
                                        live_revision = live_revision.0,
                                        "[semantic-sync] subtree delta extraction failed: {err:#}; falling back to full snapshot rebuild"
                                    );
                                    match extractor.extract(&live.stage, source) {
                                        Ok(snapshot) => (
                                            Some(SemanticSyncAction::Replace(Arc::new(snapshot))),
                                            SemanticExtractionOutcome::Fallback,
                                        ),
                                        Err(fallback_err) => {
                                            bevy::log::error!(
                                                "[semantic-sync] full snapshot fallback failed: {fallback_err:#}"
                                            );
                                            (None, SemanticExtractionOutcome::Fallback)
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    match changed_info_update(
                        &live.stage,
                        &extractor,
                        Arc::clone(&previous_snapshot),
                        &batch,
                        source.clone(),
                    ) {
                        Ok(update) => (
                            Some(SemanticSyncAction::Delta(update)),
                            SemanticExtractionOutcome::ChangedInfo,
                        ),
                        Err(err) => {
                            let reason = err.fallback_reason();
                            bevy::log::warn!(
                                target: "semantic_sync",
                                resync_fallback_reason = reason,
                                root_count = 0usize,
                                live_revision = live_revision.0,
                                "[semantic-sync] changed-info update failed: {err:#}; falling back to full snapshot rebuild"
                            );
                            match extractor.extract(&live.stage, source) {
                                Ok(snapshot) => (
                                    Some(SemanticSyncAction::Replace(Arc::new(snapshot))),
                                    SemanticExtractionOutcome::Fallback,
                                ),
                                Err(fallback_err) => {
                                    bevy::log::error!(
                                        "[semantic-sync] full snapshot fallback failed: {fallback_err:#}"
                                    );
                                    (None, SemanticExtractionOutcome::Fallback)
                                }
                            }
                        }
                    }
                }
            }
        }
    };

    if let Some(mut c) =
        world.get_resource_mut::<crate::viewport::diagnostics::performance::RendererCounters>()
    {
        c.semantic_extract_ms += extraction_started.elapsed().as_secs_f64() * 1000.0;
        match outcome {
            SemanticExtractionOutcome::InitialFailure => {
                c.semantic_initial_extraction_failures += 1;
            }
            SemanticExtractionOutcome::Fallback => c.semantic_fallback_extractions += 1,
            SemanticExtractionOutcome::Subtree => c.semantic_subtree_extractions += 1,
            SemanticExtractionOutcome::ChangedInfo => c.semantic_changed_info_updates += 1,
            SemanticExtractionOutcome::Initial => {}
        }
    }

    let Some(mut update) = update else {
        return;
    };
    let render_blob_started = Instant::now();
    let prepared_blobs =
        attach_render_blobs_to_action(world, &mut update, live_revision, root_count);
    if let Some(mut counters) =
        world.get_resource_mut::<crate::viewport::diagnostics::performance::RendererCounters>()
    {
        counters.render_blob_prepare_ms += render_blob_started.elapsed().as_secs_f64() * 1000.0;
    }
    let request_id = format!("semantic-sync-{}", live_revision.0);
    let mut published_catalogue = None;
    let submitted = match update {
        SemanticSyncAction::Replace(snapshot) => {
            let catalogue = BimReadService::new(snapshot.as_ref())
                .classification_field_catalogue(live_revision.0);
            let submitted = world
                .resource::<SemanticWorkingStore>()
                .submit_snapshot(request_id, Arc::clone(&snapshot));
            if submitted {
                queue_runtime_delivery(world, session_id, live_revision, &snapshot, prepared_blobs);
                world.resource_mut::<SemanticSyncState>().snapshot = Some(Arc::clone(&snapshot));
                if let Some(mut diff_state) = world.get_resource_mut::<SemanticDiffState>() {
                    diff_state.update_working(session_id, Arc::clone(&snapshot));
                }
                published_catalogue = Some(catalogue);
            }
            submitted
        }
        SemanticSyncAction::Delta(update) => {
            let snapshot = Arc::clone(&update.snapshot);
            let catalogue = BimReadService::new(snapshot.as_ref())
                .classification_field_catalogue(live_revision.0);
            let submitted = world
                .resource::<SemanticWorkingStore>()
                .submit_delta_with_snapshot(request_id, update.request, Arc::clone(&snapshot));
            if submitted {
                queue_runtime_delivery(world, session_id, live_revision, &snapshot, prepared_blobs);
                world.resource_mut::<SemanticSyncState>().snapshot = Some(Arc::clone(&snapshot));
                if let Some(mut diff_state) = world.get_resource_mut::<SemanticDiffState>() {
                    diff_state.update_working(session_id, Arc::clone(&snapshot));
                }
                published_catalogue = Some(catalogue);
            }
            submitted
        }
    };
    if submitted {
        if let Some(catalogue) = published_catalogue {
            classification_catalogue::publish(world, catalogue);
        }
        if let Some(mut c) =
            world.get_resource_mut::<crate::viewport::diagnostics::performance::RendererCounters>()
        {
            c.semantic_worker_submissions += 1;
        }
        let mut state = world.resource_mut::<SemanticSyncState>();
        state.session_id = Some(session_id);
        state.revision = Some(live_revision);
    } else {
        if let Some(mut c) =
            world.get_resource_mut::<crate::viewport::diagnostics::performance::RendererCounters>()
        {
            c.semantic_worker_submission_failures += 1;
        }
        bevy::log::warn!("[semantic-sync] worker channel is unavailable");
    }
}
