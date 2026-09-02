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

use bevy::prelude::Resource;
use openusd::schemas::skel::SkelAnimQuery;

use super::index::PrimEntities;
use super::performance::PerformanceCounters;
use super::stage::LiveStage;
use openusd::usd::Stage;

#[derive(Clone)]
pub(super) struct TransformBinding {
    pub(super) entity: bevy::ecs::entity::Entity,
    pub(super) xform: crate::read::xform::TransformBinding,
}

#[derive(Clone)]
pub(super) struct BlendBinding {
    pub(super) entity: bevy::ecs::entity::Entity,
    pub(super) source_indices: Vec<Option<usize>>,
}

#[derive(Clone)]
pub(super) struct SkeletonBinding {
    pub(super) animation: SkelAnimQuery,
    pub(super) joints: Vec<bevy::ecs::entity::Entity>,
    pub(super) has_translations: bool,
    pub(super) has_rotations: bool,
    pub(super) has_scales: bool,
    pub(super) blend_bindings: Vec<BlendBinding>,
}

/// Compact, typed playback targets. It is rebuilt only after projection or a
/// structural reconcile, never on every StageTime change.
#[derive(Resource, Default)]
pub(super) struct AnimationRuntime {
    pub(super) transforms: Vec<TransformBinding>,
    pub(super) skeletons: Vec<SkeletonBinding>,
}

/// Rebuild all native animation components after a projection or structural
/// reconcile. Existing joint entities are deliberately replaced so stale
/// skeleton topology cannot survive a USD resync.
pub(super) fn rebuild(world: &mut bevy::ecs::world::World, live: &LiveStage, map: &PrimEntities) {
    if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
        counters.animation_runtime_rebuilds(1);
    }
    skeleton::clear_bindings(world);
    let stage = &live.stage;
    let by_skeleton = binding::collect_bindings(stage);
    skeleton::rebuild_skeletons(world, stage, map, &by_skeleton);
    let transforms = binding::collect_transform_bindings(world, stage, map);
    let skeletons = binding::collect_skeleton_runtime(world, stage);
    world.insert_resource(AnimationRuntime {
        transforms,
        skeletons,
    });
}

/// Sample pre-bound native targets. No route registry, USD path parsing, mesh
/// reads, or mesh/material asset allocation occurs here.
pub(super) fn sample(world: &mut bevy::ecs::world::World, stage: &Stage, time: f64) {
    sampling::sample(world, stage, time);
}
