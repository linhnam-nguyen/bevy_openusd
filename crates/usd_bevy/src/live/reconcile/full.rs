use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::*;
use std::collections::HashSet;

use super::super::animation::{AnimatedPrims, prim_is_animated};
use super::super::index::PrimEntities;
use super::super::path::parent_path;
use super::super::projection::{registry_of, stage_up_axis, traverse_predicate};
use super::super::stage::LiveStage;
use super::ReconcileStats;
use crate::prim_ref::{SemanticEntityIndex, UsdPrimRef};
use crate::route::instancer_dependency::PointInstancerDependencyIndex;

/// Reconcile the projected entities against the stage's current prims (full stage).
pub(super) fn reconcile_full(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let stage = &live.stage;
    let registry = registry_of(world);
    let mut current: HashSet<String> = HashSet::new();
    if let Err(error) = stage.traverse(traverse_predicate(), |p: &openusd::sdf::Path| {
        current.insert(p.as_str().to_string());
    }) {
        bevy::log::error!(
            "[reconcile_full] stage traversal failed: {error:#}; aborting full reconcile without mutating entity mappings"
        );
        return;
    }

    let stale: Vec<(String, Entity)> = map
        .iter()
        .filter(|(p, _)| *p != "/" && !current.contains(*p))
        .map(|(p, e)| (p.to_string(), e))
        .collect();
    let despawned_count = stale.len();
    for (path, entity) in stale {
        if let Some(mut semantic_idx) = world.get_resource_mut::<SemanticEntityIndex>() {
            semantic_idx.remove_entity(entity);
        }
        if let Some(mut material_idx) =
            world.get_resource_mut::<crate::route::material::MaterialConsumerIndex>()
        {
            material_idx.remove_consumer(&path);
        }
        if let Some(mut dependencies) = world.get_resource_mut::<PointInstancerDependencyIndex>() {
            dependencies.remove_instancer(&path);
        }
        world.despawn(entity);
        map.remove_path(&path);
    }

    let root = map.entity("/").unwrap_or_else(|| {
        let r = world
            .spawn((
                UsdPrimRef {
                    path: "/".to_string(),
                },
                Transform::from_rotation(stage_up_axis(stage)),
                Visibility::default(),
            ))
            .id();
        map.insert("/", r);
        r
    });
    let mut ordered: Vec<&String> = current.iter().collect();
    ordered.sort_by_key(|p| p.matches('/').count());
    let mut animated: HashSet<String> = HashSet::new();
    let mut patched_count = 0usize;
    let mut spawned_count = 0usize;
    for path in ordered {
        let Ok(p) = openusd::sdf::path(path) else {
            continue;
        };
        if prim_is_animated(stage, &p) {
            animated.insert(path.clone());
        }
        if let Some(entity) = map.entity(path) {
            registry.patch_prim(stage, &p, world, entity, &[]);
            patched_count += 1;
        } else {
            let parent = map.entity(parent_path(path)).or(Some(root));
            let mut e = world.spawn(UsdPrimRef { path: path.clone() });
            if let Some(parent) = parent {
                e.insert(ChildOf(parent));
            }
            let entity = e.id();
            map.insert(path.clone(), entity);
            registry.project_prim(stage, &p, world, entity);
            spawned_count += 1;
        }
    }
    world.insert_resource(AnimatedPrims(animated));
    world.insert_resource(ReconcileStats {
        roots: 1,
        visited_stage_prims: current.len(),
        patched_entities: patched_count,
        spawned_entities: spawned_count,
        despawned_entities: despawned_count,
    });
}
