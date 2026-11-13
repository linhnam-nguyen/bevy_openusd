use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::*;
use std::collections::HashSet;

use super::super::animation::{AnimatedPrims, prim_is_animated};
use super::super::index::PrimEntities;
use super::super::native_animation;
use super::super::native_instance_dependency::NativeInstanceDependencyIndex;
use super::super::path::{PathStore, parent_path};
use super::super::performance::PerformanceCounters;
use super::super::projection::{registry_of, stage_up_axis, traverse_predicate};
use super::super::stage::LiveStage;
use super::{FullWork, ReconcileExecutor};
use crate::prim_ref::{SemanticEntityIndex, UsdPrimRef};
use crate::route::instancer_dependency::PointInstancerDependencyIndex;
use crate::route::remove_mesh_projection_consumer;

/// Reconcile the projected entities against the stage's current prims (full stage).
pub(super) fn reconcile_full(
    world: &mut World,
    live: &LiveStage,
    map: &mut PrimEntities,
    executor: &mut impl ReconcileExecutor,
) {
    let stage = &live.stage;
    let registry = registry_of(world);
    if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
        counters.projection_full_stage_walks(1);
    }
    let mut current: HashSet<String> = HashSet::new();
    if let Err(error) = stage.traverse(traverse_predicate(), |p: &openusd::sdf::Path| {
        current.insert(p.as_str().to_string());
    }) {
        bevy::log::error!(
            "[reconcile_full] stage traversal failed: {error:#}; aborting full reconcile without mutating entity mappings"
        );
        return;
    }

    let stale: Vec<(String, Entity)> = {
        let paths = world.resource::<PathStore>();
        map.iter(&paths)
            .filter(|(p, _)| *p != "/" && !current.contains(*p))
            .map(|(p, e)| (p.to_string(), e))
            .collect()
    };
    executor.execute_full(
        world,
        live,
        map,
        FullWork {
            registry: &registry,
            current: &current,
            stale,
        },
    );
    world.init_resource::<NativeInstanceDependencyIndex>();
    let rebuild = world.resource_scope(
        |world, mut dependencies: Mut<NativeInstanceDependencyIndex>| {
            let mut paths = world.resource_mut::<PathStore>();
            dependencies.rebuild(&mut paths, stage)
        },
    );
    if let Err(error) = rebuild {
        bevy::log::warn!("[reconcile_full] native instance dependency rebuild failed: {error:#}");
    }
}

pub(super) fn execute_full_work(
    world: &mut World,
    live: &LiveStage,
    map: &mut PrimEntities,
    work: FullWork<'_>,
    executor: &mut impl ReconcileExecutor,
) {
    let FullWork {
        registry,
        current,
        stale,
    } = work;
    let stage = &live.stage;
    for (path, entity) in stale {
        if let Some(mut semantic_idx) = world.get_resource_mut::<SemanticEntityIndex>() {
            semantic_idx.remove_entity(entity);
        }
        if let Some(mut material_idx) =
            world.get_resource_mut::<crate::route::material::MaterialConsumerIndex>()
        {
            material_idx.remove_consumer(&path);
        }
        if world.contains_resource::<PointInstancerDependencyIndex>() {
            world.resource_scope(
                |world, mut dependencies: Mut<PointInstancerDependencyIndex>| {
                    let paths = world.resource::<PathStore>();
                    dependencies.remove_instancer(&paths, &path);
                },
            );
        }
        remove_mesh_projection_consumer(world, entity);
        world.despawn(entity);
        let paths = world.resource::<PathStore>();
        map.remove_path(&paths, &path);
        if world.contains_resource::<NativeInstanceDependencyIndex>() {
            world.resource_scope(
                |world, mut dependencies: Mut<NativeInstanceDependencyIndex>| {
                    let paths = world.resource::<PathStore>();
                    dependencies.remove_proxy(&paths, &path);
                },
            );
        }
    }

    let root = {
        let paths = world.resource::<PathStore>();
        map.entity(&paths, "/")
    }
    .unwrap_or_else(|| {
        let r = world
            .spawn((
                UsdPrimRef {
                    path: "/".to_string(),
                },
                Transform::from_rotation(stage_up_axis(stage)),
                Visibility::default(),
            ))
            .id();
        world.resource_scope(|_world, mut paths: Mut<PathStore>| {
            map.insert(&mut paths, "/", r);
        });
        r
    });
    let mut ordered: Vec<&String> = current.iter().collect();
    ordered.sort_by_key(|p| p.matches('/').count());
    let mut animated: HashSet<String> = HashSet::new();
    for path in ordered {
        let Ok(p) = openusd::sdf::path(path) else {
            continue;
        };
        if prim_is_animated(stage, &p) {
            animated.insert(path.clone());
        }
        let existing = {
            let paths = world.resource::<PathStore>();
            map.entity(&paths, path)
        };
        if let Some(entity) = existing {
            executor.patch_prim(registry, stage, &p, world, entity, &[]);
        } else {
            let parent = {
                let paths = world.resource::<PathStore>();
                map.entity(&paths, parent_path(path)).or(Some(root))
            };
            let mut e = world.spawn(UsdPrimRef { path: path.clone() });
            if let Some(parent) = parent {
                e.insert(ChildOf(parent));
            }
            let entity = e.id();
            world.resource_scope(|_world, mut paths: Mut<PathStore>| {
                map.insert(&mut paths, path, entity);
            });
            executor.project_prim(registry, stage, &p, world, entity);
        }
    }
    world.insert_resource(AnimatedPrims(animated));
    native_animation::rebuild(world, live, map);
}
