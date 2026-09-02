mod full;
mod subtree;

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use super::change::StageChangeBatch;
use super::index::PrimEntities;
use super::native_instance_dependency::NativeInstanceDependencyIndex;
use super::path::{PathId, PathStore, prim_of, property_of, validate_prim_path};
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

/// Compact, batch-local representation of sparse changed-info work.
///
/// Prim paths are interned once into [`PathId`]s. Property names remain
/// borrowed from the authoritative change batch, so the plan owns neither
/// repeated path strings nor cloned property strings. The per-prim vectors
/// are deduplicated before route patching and can be reused for dependency
/// fanout.
#[derive(Default)]
struct ChangePlan<'a> {
    changed: Vec<PrimChange<'a>>,
    by_path: HashMap<PathId, usize>,
    changed_info_count: usize,
}

struct PrimChange<'a> {
    path: PathId,
    properties: Vec<&'a str>,
}

impl<'a> ChangePlan<'a> {
    fn from_changed_info<I>(
        paths: &mut PathStore,
        changed_info: I,
        suppressed: &HashSet<String>,
    ) -> Self
    where
        I: IntoIterator<Item = &'a String>,
    {
        let mut plan = Self::default();
        for info_path in changed_info {
            plan.changed_info_count += 1;
            let prim = prim_of(info_path);
            if suppressed.contains(prim) {
                continue;
            }
            let path = paths.lookup(prim).unwrap_or_else(|| paths.intern(prim));
            plan.add(path, property_of(info_path));
        }
        plan
    }

    fn add(&mut self, path: PathId, property: Option<&'a str>) {
        let index = if let Some(index) = self.by_path.get(&path).copied() {
            index
        } else {
            let index = self.changed.len();
            self.changed.push(PrimChange {
                path,
                properties: Vec::new(),
            });
            self.by_path.insert(path, index);
            index
        };
        if let Some(property) = property
            && !self.changed[index]
                .properties
                .iter()
                .any(|existing| *existing == property)
        {
            self.changed[index].properties.push(property);
        }
    }

    fn add_properties<I>(&mut self, path: PathId, properties: I)
    where
        I: IntoIterator<Item = &'a str>,
    {
        let index = if let Some(index) = self.by_path.get(&path).copied() {
            index
        } else {
            let index = self.changed.len();
            self.changed.push(PrimChange {
                path,
                properties: Vec::new(),
            });
            self.by_path.insert(path, index);
            index
        };
        for property in properties {
            if !self.changed[index]
                .properties
                .iter()
                .any(|existing| *existing == property)
            {
                self.changed[index].properties.push(property);
            }
        }
    }
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
pub(super) fn apply_sparse_changed_info<'a, I>(
    world: &mut World,
    live: &LiveStage,
    map: &mut PrimEntities,
    changed_info: I,
) where
    I: IntoIterator<Item = &'a String>,
{
    let registry = registry_of(world);
    let suppressed = live.take_suppressed();
    let plan = {
        let mut paths = world.resource_mut::<PathStore>();
        ChangePlan::from_changed_info(&mut paths, changed_info, &suppressed)
    };
    if plan.changed.is_empty() {
        return;
    }
    let mut native_dependents = ChangePlan::default();
    let mut dependent_instancers: HashSet<PathId> = HashSet::new();
    if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
        counters.reconcile_changed_properties(plan.changed_info_count as u64);
    }

    // Dependency queries are keyed by owning prim, not by individual changed
    // property. Resolve each distinct prim once, then fan its already-planned
    // property slice out to native dependents.
    for change in &plan.changed {
        let (instancers, native) = {
            let paths = world.resource::<PathStore>();
            let Some(prim) = paths.path(change.path) else {
                continue;
            };
            let instancers = world
                .get_resource::<PointInstancerDependencyIndex>()
                .map(|index| index.dependents_for_path(&paths, prim))
                .unwrap_or_default();
            let native = world
                .get_resource::<NativeInstanceDependencyIndex>()
                .map(|index| index.dependents_for_path(&paths, prim))
                .unwrap_or_default();
            (instancers, native)
        };
        dependent_instancers.extend(instancers);
        for dependent in native {
            native_dependents.add_properties(dependent, change.properties.iter().copied());
        }
        if world.contains_resource::<PointInstancerDependencyIndex>()
            || world.contains_resource::<NativeInstanceDependencyIndex>()
        {
            if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
                counters.reconcile_dependency_queries(1);
            }
        }
    }

    if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
        counters.reconcile_distinct_prims(plan.changed.len() as u64);
    }

    let mut patched_count = 0;
    for change in plan.changed {
        let Some(path) = world
            .resource::<PathStore>()
            .path(change.path)
            .and_then(|path| openusd::sdf::path(path).ok())
        else {
            continue;
        };
        let Some(entity) = map.entity_id(change.path) else {
            continue;
        };
        registry.patch_prim(&live.stage, &path, world, entity, &change.properties);
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
    instancers: HashSet<PathId>,
) -> usize {
    let mut patched = 0;
    for instancer in instancers {
        let Some(entity) = map.entity_id(instancer) else {
            continue;
        };
        let Some(path) = world
            .resource::<PathStore>()
            .path(instancer)
            .and_then(|path| openusd::sdf::path(path).ok())
        else {
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
    dependents: ChangePlan<'_>,
) -> usize {
    let mut patched = 0;
    for change in dependents.changed {
        let Some(entity) = map.entity_id(change.path) else {
            continue;
        };
        let Some(path) = world
            .resource::<PathStore>()
            .path(change.path)
            .and_then(|path| openusd::sdf::path(path).ok())
        else {
            continue;
        };
        registry.patch_prim(stage, &path, world, entity, &change.properties);
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
    crate::route::geom::note_hierarchy_metadata_revision(world, batch.revision.0);
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
        let resync_roots = batch.resync_roots();
        if !resync_roots.is_empty() {
            let instancer_dependencies = world
                .get_resource::<PointInstancerDependencyIndex>()
                .zip(world.get_resource::<PathStore>())
                .map(|(index, paths)| {
                    resync_roots
                        .iter()
                        .flat_map(|path| index.dependents_for_resync_root(paths, path))
                        .collect::<HashSet<_>>()
                })
                .unwrap_or_default();
            let native_dependencies = {
                let mut plan = ChangePlan::default();
                if let (Some(index), Some(paths)) = (
                    world.get_resource::<NativeInstanceDependencyIndex>(),
                    world.get_resource::<PathStore>(),
                ) {
                    for proxy in resync_roots
                        .iter()
                        .flat_map(|path| index.dependents_for_resync_root(paths, path))
                    {
                        plan.add(proxy, None);
                    }
                }
                plan
            };
            if world.contains_resource::<PointInstancerDependencyIndex>()
                || world.contains_resource::<NativeInstanceDependencyIndex>()
            {
                if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
                    counters.reconcile_dependency_queries(resync_roots.len() as u64);
                }
            }
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

    apply_sparse_changed_info(
        world,
        live,
        map,
        batch
            .changes
            .iter()
            .flat_map(|change| change.changed_info.iter()),
    );
    cleanup_retired_materials(world);
}
