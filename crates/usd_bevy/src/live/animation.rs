use bevy::prelude::*;
use openusd::usd::Stage;
use std::collections::HashSet;

use super::index::PrimEntities;
use super::performance::PerformanceCounters;
use super::projection::registry_of;
use super::stage::LiveStage;
use crate::route::StageTime;

/// Prim paths that have at least one time-sampled (animated) attribute — the
/// set the animation resampler revisits when [`StageTime`] changes. Computed
/// once at projection.
#[derive(Resource, Default, Clone)]
pub struct AnimatedPrims(pub HashSet<String>);

/// The [`StageTime`] the projected entities were last sampled at, so the
/// resampler only reruns when the time actually moves.
#[derive(Resource, Default)]
pub(super) struct SampledTime(pub(super) Option<f64>);

/// Whether `prim` animates: it has a time-sampled attribute of its own, or it
/// is a skinned mesh driven by a time-varying SkelAnimation (whose samples live
/// on a different prim).
pub(super) fn prim_is_animated(stage: &Stage, path: &openusd::sdf::Path) -> bool {
    let own = stage
        .prim(path.clone())
        .attributes()
        .map(|attrs| {
            attrs.iter().any(|a| {
                a.time_sample_times()
                    .map(|times| !times.is_empty())
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    own || crate::read::skel::skin_is_time_varying(stage, path)
        || crate::read::skel::blend_is_time_varying(stage, path)
}

/// Resample animated prims when [`StageTime`] moves. Only revisits the prims
/// that actually carry time samples ([`AnimatedPrims`]), re-patching them at
/// the new time (the routes read `StageTime` when resolving values).
pub(super) fn resample_animation_system(world: &mut World) {
    if world.get_non_send::<LiveStage>().is_none() {
        return;
    }
    let current = world.get_resource::<StageTime>().map(|t| t.current);
    let last = world.get_resource::<SampledTime>().and_then(|t| t.0);
    if current == last {
        return; // time hasn't moved
    }
    if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
        counters.stage_time_changes(1);
    }
    let animated = world
        .get_resource::<AnimatedPrims>()
        .cloned()
        .unwrap_or_default();
    if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
        counters.animation_runtime_samples(1);
    }
    if animated.0.is_empty() {
        // Nothing animated; still record the time so we don't re-check.
        if let Some(mut sampled) = world.get_resource_mut::<SampledTime>() {
            sampled.0 = current;
        }
        return;
    }
    let Some(live) = world.remove_non_send::<LiveStage>() else {
        return;
    };
    let map = world.remove_resource::<PrimEntities>().unwrap_or_default();
    let registry = registry_of(world);
    for path in &animated.0 {
        if let Some(entity) = map.entity(path) {
            if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
                counters.animation_usd_path_parses(1);
            }
            if let Ok(p) = openusd::sdf::path(path) {
                if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
                    counters.animation_generic_patch_calls(1);
                }
                // patch_prim resolves values at the world's StageTime.
                registry.patch_prim(&live.stage, &p, world, entity, &[]);
            }
        }
    }
    world.insert_resource(map);
    world.insert_non_send(live);
    if let Some(mut sampled) = world.get_resource_mut::<SampledTime>() {
        sampled.0 = current;
    }
}
