use bevy::prelude::*;
use openusd::usd::Stage;
use std::collections::HashSet;

use super::native_animation;
use super::performance::PerformanceCounters;
use super::stage::LiveStage;
use crate::route::StageTime;

/// Composed prim paths that own or consume time-sampled data.
/// Updated during projection and reconciliation.
#[derive(Resource, Default, Clone)]
pub struct AnimatedPrims(pub HashSet<String>);

/// The playback time and visual composition last sampled together.
#[derive(Resource, Default)]
pub(super) struct SampledTime {
    time: Option<f64>,
    pub(super) context: Option<super::stage::VisualContextId>,
}

/// Whether the composed prim has time-sampled attributes or consumes
/// time-sampled data from a skeleton animation.
pub(super) fn prim_is_animated(stage: &Stage, path: &openusd::sdf::Path) -> bool {
    let own = stage
        .prim(path.clone())
        .ok()
        .and_then(|prim| prim.attributes().ok())
        .map(|attrs| {
            attrs.iter().any(|a| {
                a.might_be_time_varying().unwrap_or(false)
                    || a.num_time_samples().is_ok_and(|count| count != 0)
            })
        })
        .unwrap_or(false);
    own || crate::read::skel::deformation_is_time_varying(stage, path)
}

/// Sample the pre-bound consumers when time or visual composition changes.
pub(super) fn resample_animation_system(world: &mut World) {
    let Some(context) = world
        .get_non_send::<LiveStage>()
        .map(LiveStage::visual_context_id)
    else {
        return;
    };
    let current = world.get_resource::<StageTime>().map(|t| t.current);
    let sampled = world.resource::<SampledTime>();
    let time_changed = current != sampled.time;
    if !time_changed && sampled.context == Some(context) {
        return;
    }
    if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
        if time_changed {
            counters.stage_time_changes(1);
        }
        counters.animation_runtime_samples(1);
    }
    let Some(live) = world.remove_non_send::<LiveStage>() else {
        return;
    };
    native_animation::sample(world, &live, current.unwrap_or_default());
    world.insert_non_send(live);
    *world.resource_mut::<SampledTime>() = SampledTime {
        time: current,
        context: Some(context),
    };
}
