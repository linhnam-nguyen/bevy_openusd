//! Projection-time animation bindings.
//!
//! This module owns the typed animation boundary introduced by M8-OR3-C1.
//! USD paths, skeleton joint entities, inverse bind poses, and morph target
//! names are resolved while a stage is projected. StageTime sampling then
//! updates only existing Bevy components.

mod binding;
mod mesh_binding;
mod sampling;
mod skeleton;

use openusd_schemas::skel::AnimQuery;

use super::index::PrimEntities;
use super::performance::PerformanceCounters;
use super::stage::{LiveStage, VisualContextId};

#[derive(Clone)]
pub(super) struct TransformBinding {
    pub(super) entity: bevy::ecs::entity::Entity,
    pub(super) xform: crate::read::xform::TransformBinding,
}

#[derive(Clone)]
pub(super) struct VisibilityBinding {
    pub(super) entity: bevy::ecs::entity::Entity,
    pub(super) path: openusd::sdf::Path,
}

#[derive(Clone)]
pub(super) struct BlendBinding {
    pub(super) entity: bevy::ecs::entity::Entity,
    pub(super) source_indices: Vec<Option<usize>>,
}

#[derive(Clone)]
pub(super) struct SkeletonBinding {
    pub(super) joints: Vec<Option<bevy::ecs::entity::Entity>>,
    pub(super) has_translations: bool,
    pub(super) has_rotations: bool,
    pub(super) has_scales: bool,
}

pub(super) struct SharedPose {
    pub(super) animation: AnimQuery,
    pub(super) skeletons: Vec<SkeletonBinding>,
    pub(super) blend_bindings: Vec<BlendBinding>,
}

#[derive(bevy::prelude::Component)]
pub(super) struct CpuDeformBinding(openusd::sdf::Path);

/// Compact, typed playback targets owned by one visual composition.
#[derive(Default)]
pub(super) struct AnimationRuntime {
    context: Option<VisualContextId>,
    pub(super) transforms: Vec<TransformBinding>,
    pub(super) visibilities: Vec<VisibilityBinding>,
    pub(super) poses: Vec<SharedPose>,
}

/// Rebuild all native animation components after a projection or structural
/// reconcile. Existing joint entities are deliberately replaced so stale
/// skeleton topology cannot survive a USD resync.
pub(super) fn rebuild(world: &mut bevy::ecs::world::World, live: &LiveStage, map: &PrimEntities) {
    if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
        counters.animation_runtime_rebuilds(1);
    }
    skeleton::clear_bindings(world);
    let stage = &live.projection_stage();
    let by_skeleton = binding::collect_bindings(stage);
    skeleton::rebuild_skeletons(world, stage, map, &by_skeleton);
    let transforms = binding::collect_transform_bindings(world, stage, map);
    let visibilities = binding::collect_visibility_bindings(world, stage, map);
    let poses = binding::collect_skeleton_runtime(world, stage);
    world.insert_non_send(AnimationRuntime {
        context: Some(live.visual_context_id()),
        transforms,
        visibilities,
        poses,
    });
    if let Some(mut sampled) = world.get_resource_mut::<super::animation::SampledTime>() {
        sampled.context = None;
    }
}

/// Sample pre-bound native targets. Ordinary clock ticks only update components.
pub(super) fn sample(world: &mut bevy::ecs::world::World, live: &LiveStage, time: f64) {
    let stage = live.projection_stage();
    let context = Some(live.visual_context_id());
    if world
        .get_non_send::<AnimationRuntime>()
        .is_some_and(|runtime| runtime.context.is_some() && runtime.context != context)
        && world.contains_resource::<PrimEntities>()
    {
        // Sparse changes preserve entity bindings but may alter the composed
        // spec stacks or time-sample status. Reuse the existing native joints.
        let (transforms, visibilities) =
            world.resource_scope(|world, map: bevy::prelude::Mut<PrimEntities>| {
                (
                    binding::collect_transform_bindings(world, &stage, &map),
                    binding::collect_visibility_bindings(world, &stage, &map),
                )
            });
        let poses = binding::collect_skeleton_runtime(world, &stage);
        world.insert_non_send(AnimationRuntime {
            context,
            transforms,
            visibilities,
            poses,
        });
    }
    sampling::sample(world, &stage, time);
}
