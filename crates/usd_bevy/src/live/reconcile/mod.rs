mod full;
mod subtree;

use bevy::prelude::*;
use std::collections::HashMap;

use super::change::StageChangeBatch;
use super::index::PrimEntities;
use super::path::{prim_of, property_of, validate_prim_path};
use super::projection::registry_of;
use super::stage::LiveStage;
use full::reconcile_full;
use subtree::reconcile_subtrees;

/// Internal work counters for the most recent reconcile pass.
///
/// Test and profiling suites use this to verify work reduction during subtree
/// resync without relying on noisy timing assertions.
#[derive(Resource, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ReconcileStats {
    pub(crate) roots: usize,
    pub(crate) visited_stage_prims: usize,
    pub(crate) patched_entities: usize,
    pub(crate) spawned_entities: usize,
    pub(crate) despawned_entities: usize,
}

/// Drain the change queue and reproject affected entities.
pub fn apply_changes(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let Some(batch) = live.drain_change_batch() else {
        return;
    };
    apply_change_batch(world, live, map, &batch);
}

/// Sparse property patch applied per owning prim. Each prim's registered route
/// runs with `changed` pointing to only the properties modified in the batch.
pub(super) fn apply_sparse_changed_info(
    world: &mut World,
    live: &LiveStage,
    map: &mut PrimEntities,
    changed_info: &[String],
) {
    if changed_info.is_empty() {
        return;
    }
    let registry = registry_of(world);
    let suppressed = live.take_suppressed();
    let mut per_prim: HashMap<String, Vec<String>> = HashMap::new();
    for prop_path in changed_info {
        let prim = prim_of(prop_path);
        let entry = per_prim.entry(prim.to_string()).or_default();
        if let Some(prop) = property_of(prop_path) {
            entry.push(prop.to_string());
        }
    }

    for (prim, props) in per_prim {
        if suppressed.contains(&prim) {
            continue;
        }
        let Ok(p) = openusd::sdf::path(&prim) else {
            continue;
        };
        let Some(entity) = map.entity(&prim) else {
            continue;
        };
        let prop_refs: Vec<&str> = props.iter().map(String::as_str).collect();
        registry.patch_prim(&live.stage, &p, world, entity, &prop_refs);
    }
}

/// Reproject one already-drained batch without touching the live-stage queue.
pub fn apply_change_batch(
    world: &mut World,
    live: &LiveStage,
    map: &mut PrimEntities,
    batch: &StageChangeBatch,
) {
    if batch.is_empty() {
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
            if let Err(err) = validate_prim_path(r) {
                bevy::log::warn!(
                    target: "usd_bevy",
                    resync_fallback_reason = "unnormalizable_root",
                    root_count = all_resynced.len(),
                    live_revision = batch.revision.0,
                    "[subtree-reconcile] root '{r}' cannot represent a safe OpenUSD prim path: {err:#}; falling back to full reconcile"
                );
                unnormalizable = true;
                break;
            }
        }

        if unnormalizable {
            reconcile_full(world, live, map);
        } else {
            let roots = batch.resync_roots();
            if roots.contains(&"/".to_string()) || roots.is_empty() {
                bevy::log::warn!(
                    target: "usd_bevy",
                    resync_fallback_reason = "root_is_stage_root_or_empty",
                    root_count = roots.len(),
                    live_revision = batch.revision.0,
                    "[subtree-reconcile] stage root '/' or empty roots in batch; falling back to full reconcile"
                );
                reconcile_full(world, live, map);
            } else {
                reconcile_subtrees(world, live, map, &roots, batch.revision);
            }
        }
        let unshaded = batch.unshaded_changed_info();
        apply_sparse_changed_info(world, live, map, &unshaded);
        return;
    }

    let all_changed_info: Vec<String> = batch
        .changes
        .iter()
        .flat_map(|c| &c.changed_info)
        .cloned()
        .collect();
    apply_sparse_changed_info(world, live, map, &all_changed_info);
}
