mod full;
mod subtree;

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use super::change::StageChangeBatch;
use super::index::PrimEntities;
use super::native_instance_dependency::NativeInstanceDependencyIndex;
use super::path::{prim_of, property_of, validate_prim_path};
use super::performance::PerformanceCounters;
use super::projection::registry_of;
use super::stage::LiveStage;
use crate::route::instancer_dependency::PointInstancerDependencyIndex;
use crate::route::material::cleanup_retired_materials;
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
    let mut native_dependents: HashMap<String, Vec<String>> = HashMap::new();
    let mut dependent_instancers = HashSet::new();
    if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
        counters.reconcile_changed_properties(changed_info.len() as u64);
    }
    for prop_path in changed_info {
        let prim = prim_of(prop_path);
        if suppressed.contains(prim) {
            continue;
        }
        let entry = per_prim.entry(prim.to_string()).or_default();
        if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
            counters.reconcile_string_materializations(1);
        }
        if let Some(prop) = property_of(prop_path) {
            entry.push(prop.to_string());
            if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
                counters.reconcile_string_materializations(1);
            }
        }
        let has_instancer_index = world.contains_resource::<PointInstancerDependencyIndex>();
        if has_instancer_index {
            if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
                counters.reconcile_dependency_queries(1);
            }
        }
        if let Some(index) = world.get_resource::<PointInstancerDependencyIndex>() {
            dependent_instancers.extend(index.dependents_for_path(prim));
        }
        let has_native_index = world.contains_resource::<NativeInstanceDependencyIndex>();
        if has_native_index {
            if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
                counters.reconcile_dependency_queries(1);
            }
        }
        if let Some(index) = world.get_resource::<NativeInstanceDependencyIndex>() {
            for dependent in index.dependents_for_path(prim) {
                if let Some(property) = property_of(prop_path) {
                    native_dependents
                        .entry(dependent)
                        .or_default()
                        .push(property.to_string());
                } else {
                    native_dependents.entry(dependent).or_default();
                }
            }
        }
    }

    if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
        counters.reconcile_distinct_prims(per_prim.len() as u64);
    }

    let mut patched_count = 0;
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
        patched_count += 1;
    }
    patched_count +=
        apply_prototype_dependents(world, &live.stage, map, &registry, dependent_instancers);
    patched_count +=
        apply_native_instance_dependents(world, &live.stage, map, &registry, native_dependents);
    world.insert_resource(ReconcileStats {
        visited_stage_prims: patched_count,
        patched_entities: patched_count,
        ..Default::default()
    });
}

fn apply_prototype_dependents(
    world: &mut World,
    stage: &openusd::usd::Stage,
    map: &PrimEntities,
    registry: &crate::route::SchemaRegistry,
    instancers: HashSet<String>,
) -> usize {
    let mut patched = 0;
    for instancer in instancers {
        let Some(entity) = map.entity(&instancer) else {
            continue;
        };
        let Ok(path) = openusd::sdf::path(&instancer) else {
            continue;
        };
        registry.patch_prim(stage, &path, world, entity, &["prototype_dependency"]);
        patched += 1;
    }
    patched
}

fn apply_native_instance_dependents(
    world: &mut World,
    stage: &openusd::usd::Stage,
    map: &PrimEntities,
    registry: &crate::route::SchemaRegistry,
    dependents: HashMap<String, Vec<String>>,
) -> usize {
    let mut patched = 0;
    for (proxy, properties) in dependents {
        let Some(entity) = map.entity(&proxy) else {
            continue;
        };
        let Ok(path) = openusd::sdf::path(&proxy) else {
            continue;
        };
        let property_refs: Vec<&str> = properties.iter().map(String::as_str).collect();
        registry.patch_prim(stage, &path, world, entity, &property_refs);
        patched += 1;
    }
    patched
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
        let resynced_paths = batch
            .changes
            .iter()
            .flat_map(|change| change.resynced.iter())
            .map(String::as_str)
            .collect::<Vec<_>>();
        if !resynced_paths.is_empty() {
            let instancer_dependencies = world
                .get_resource::<PointInstancerDependencyIndex>()
                .map(|index| {
                    resynced_paths
                        .iter()
                        .flat_map(|path| index.dependents_for_resync_root(path))
                        .collect::<HashSet<_>>()
                })
                .unwrap_or_default();
            let native_dependencies = world
                .get_resource::<NativeInstanceDependencyIndex>()
                .map(|index| {
                    resynced_paths
                        .iter()
                        .flat_map(|path| index.dependents_for_resync_root(path))
                        .map(|proxy| (proxy, Vec::new()))
                        .collect::<HashMap<_, _>>()
                })
                .unwrap_or_default();
            let registry = registry_of(world);
            apply_prototype_dependents(world, &live.stage, map, &registry, instancer_dependencies);
            apply_native_instance_dependents(
                world,
                &live.stage,
                map,
                &registry,
                native_dependencies,
            );
        }
        cleanup_retired_materials(world);
        return;
    }

    let all_changed_info: Vec<String> = batch
        .changes
        .iter()
        .flat_map(|c| &c.changed_info)
        .cloned()
        .collect();
    apply_sparse_changed_info(world, live, map, &all_changed_info);
    cleanup_retired_materials(world);
}
