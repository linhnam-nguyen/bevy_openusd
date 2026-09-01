use bevy::prelude::*;

use super::change::PendingStageChanges;
use super::index::PrimEntities;
use super::path::PathStore;
use super::projection::registry_of;
use super::reconcile::apply_change_batch;
use super::stage::LiveStage;

/// The [`DisplayPurposes`] the projected entities were last filtered against,
/// so the purpose reprojector only reruns when the toggle actually changes.
#[derive(Resource, Default)]
pub(super) struct AppliedPurposes(pub(super) Option<crate::route::DisplayPurposes>);

/// Re-filter every prim's visibility when [`crate::route::DisplayPurposes`] changes.
pub(super) fn apply_display_purposes_system(world: &mut World) {
    if world.get_non_send::<LiveStage>().is_none() {
        return;
    }
    let current = world
        .get_resource::<crate::route::DisplayPurposes>()
        .copied();
    let last = world.get_resource::<AppliedPurposes>().and_then(|a| a.0);
    if current == last {
        return;
    }
    let Some(live) = world.remove_non_send::<LiveStage>() else {
        return;
    };
    let map = world.remove_resource::<PrimEntities>().unwrap_or_default();
    let registry = registry_of(world);
    let entries: Vec<(String, Entity)> = {
        let paths = world.resource::<PathStore>();
        map.iter(&paths).map(|(p, e)| (p.to_string(), e)).collect()
    };
    for (path, entity) in entries {
        if let Ok(p) = openusd::sdf::path(&path) {
            registry.patch_prim(&live.stage, &p, world, entity, &["purpose"]);
        }
    }
    world.insert_resource(map);
    world.insert_non_send(live);
    if let Some(mut applied) = world.get_resource_mut::<AppliedPurposes>() {
        applied.0 = current;
    }
}

/// Drain the live stage's change queue once and publish the batch.
pub(super) fn drain_stage_changes_system(world: &mut World) {
    let batch = world
        .get_non_send::<LiveStage>()
        .and_then(LiveStage::drain_change_batch);
    world.resource_mut::<PendingStageChanges>().batch = batch;
}

/// Reproject the batch published by [`drain_stage_changes_system`].
pub(super) fn reproject_from_batch_system(world: &mut World) {
    let batch = world.resource::<PendingStageChanges>().batch.clone();
    let Some(batch) = batch else {
        return;
    };
    let Some(live) = world.remove_non_send::<LiveStage>() else {
        return;
    };
    let mut map = world.remove_resource::<PrimEntities>().unwrap_or_default();
    apply_change_batch(world, &live, &mut map, &batch);
    world.insert_resource(map);
    world.insert_non_send(live);
}
