use std::collections::{HashMap, HashSet};
use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::*;

use crate::prim_ref::{SemanticEntityIndex, UsdPrimRef};
use super::ReconcileStats;
use super::full::reconcile_full;
use super::super::animation::{AnimatedPrims, prim_is_animated};
use super::super::change::LiveRevision;
use super::super::index::PrimEntities;
use super::super::path::{is_descendant_or_self, parent_path};
use super::super::projection::{collect_stage_subtree_paths, registry_of};
use super::super::stage::LiveStage;

/// Reconcile specific subtrees against the stage's current prims.
pub(super) fn reconcile_subtrees(
    world: &mut World,
    live: &LiveStage,
    map: &mut PrimEntities,
    roots: &[String],
    revision: LiveRevision,
) {
    let stage = &live.stage;
    let registry = registry_of(world);
    let Some(root_entity) = map.entity("/") else {
        bevy::log::warn!(
            target: "usd_bevy",
            resync_fallback_reason = "root_entity_missing",
            root_count = roots.len(),
            live_revision = revision.0,
            "[subtree-reconcile] stage root '/' missing from PrimEntities; falling back to full reconcile"
        );
        reconcile_full(world, live, map);
        return;
    };

    let mut old_entities: HashMap<String, Entity> = HashMap::new();
    for root in roots {
        for (path, entity) in map.subtree(root) {
            if path != "/" {
                old_entities.insert(path, entity);
            }
        }
    }

    let mut current_paths: HashSet<String> = HashSet::new();
    for root in roots {
        match collect_stage_subtree_paths(stage, root) {
            Ok(paths) => {
                current_paths.extend(paths);
            }
            Err(error) => {
                bevy::log::warn!(
                    target: "usd_bevy",
                    resync_fallback_reason = "subtree_collection_failed",
                    root_count = roots.len(),
                    live_revision = revision.0,
                    "[subtree-reconcile] collection failed for root '{root}': {error:#}; falling back to full reconcile"
                );
                reconcile_full(world, live, map);
                return;
            }
        }
    }

    // 1. Preflight parent integrity for all new prims (current_paths - old_paths) BEFORE any mutations
    let mut added: Vec<String> = current_paths
        .iter()
        .filter(|path| !old_entities.contains_key(*path))
        .cloned()
        .collect();
    added.sort_by(|a, b| a.matches('/').count().cmp(&b.matches('/').count()));

    for path in &added {
        let parent_str = parent_path(path);
        let parent_will_exist = if parent_str == "/" {
            true
        } else {
            current_paths.contains(parent_str) || map.entity(parent_str).is_some()
        };

        if !parent_will_exist {
            bevy::log::warn!(
                target: "usd_bevy",
                resync_fallback_reason = "unresolved_parent",
                root_count = roots.len(),
                live_revision = revision.0,
                "[subtree-reconcile] parent '{parent_str}' for new prim '{path}' is missing; falling back to full reconcile"
            );
            reconcile_full(world, live, map);
            return;
        }
    }

    // 2. Despawn removed prims (old_paths - current_paths), deepest first
    let mut removed: Vec<(String, Entity)> = old_entities
        .iter()
        .filter(|(path, _)| !current_paths.contains(*path))
        .map(|(path, entity)| (path.clone(), *entity))
        .collect();
    removed.sort_by(|(a, _), (b, _)| b.matches('/').count().cmp(&a.matches('/').count()));

    let despawned_count = removed.len();
    for (path, entity) in removed {
        if let Some(mut semantic_idx) = world.get_resource_mut::<SemanticEntityIndex>() {
            semantic_idx.remove_entity(entity);
        }
        world.despawn(entity);
        map.remove_path(&path);
    }

    // 3. Spawn new prims (current_paths - old_paths), shallowest first
    let mut spawned_count = 0usize;
    for path in &added {
        let Ok(p) = openusd::sdf::path(path) else {
            continue;
        };
        let parent_str = parent_path(path);
        let parent = if parent_str == "/" {
            Some(root_entity)
        } else {
            map.entity(parent_str)
        };
        let Some(parent) = parent else {
            bevy::log::error!(
                target: "usd_bevy",
                resync_fallback_reason = "missing_parent_entity",
                root_count = roots.len(),
                live_revision = revision.0,
                "[subtree-reconcile] parent '{parent_str}' missing during spawn for '{path}'; falling back to full reconcile"
            );
            reconcile_full(world, live, map);
            return;
        };
        let mut e = world.spawn(UsdPrimRef { path: path.clone() });
        e.insert(ChildOf(parent));
        let entity = e.id();
        map.insert(path.clone(), entity);
        registry.project_prim(stage, &p, world, entity);
        spawned_count += 1;
    }

    // 4. Repatch existing prims (current_paths ∩ old_paths)
    let mut patched_count = 0usize;
    for path in &current_paths {
        if let Some(&entity) = old_entities.get(path) {
            if let Ok(p) = openusd::sdf::path(path) {
                registry.patch_prim(stage, &p, world, entity, &[]);
                patched_count += 1;
            }
        }
    }

    // 5. Maintain AnimatedPrims for the affected subtrees
    if let Some(mut animated_res) = world.get_resource_mut::<AnimatedPrims>() {
        animated_res.0.retain(|anim_path| {
            !roots
                .iter()
                .any(|root| is_descendant_or_self(root, anim_path))
        });
        for path in &current_paths {
            if let Ok(p) = openusd::sdf::path(path) {
                if prim_is_animated(stage, &p) {
                    animated_res.0.insert(path.clone());
                }
            }
        }
    }

    world.insert_resource(ReconcileStats {
        roots: roots.len(),
        visited_stage_prims: current_paths.len(),
        patched_entities: patched_count,
        spawned_entities: spawned_count,
        despawned_entities: despawned_count,
    });
}
