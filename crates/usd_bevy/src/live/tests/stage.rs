use bevy::prelude::*;
use openusd::usd::Stage;

use crate::live::{
    LiveRevision, LiveStage, LiveStagePlugin, PendingStageChanges, StageChange, StageChangeBatch,
};

#[test]
fn non_empty_drains_advance_revision_once_and_are_not_replayed() {
    let stage = Stage::builder()
        .in_memory("live-revision.usda")
        .expect("in-memory stage");
    let live = LiveStage::new(stage);

    live.enqueue_resync("/World");
    let first = live.drain_change_batch().expect("first batch");
    assert_eq!(first.revision, LiveRevision(1));
    assert_eq!(live.current_revision(), LiveRevision(1));
    assert_eq!(first.changes.len(), 1);
    assert!(live.drain_change_batch().is_none());

    live.enqueue_resync("/World/Chair");
    let second = live.drain_change_batch().expect("second batch");
    assert_eq!(second.revision, LiveRevision(2));
    assert_eq!(second.changes[0].resynced, vec!["/World/Chair".to_string()]);
}

#[test]
fn pending_batch_is_readable_without_consuming_it() {
    let batch = StageChangeBatch {
        revision: LiveRevision(7),
        changes: vec![StageChange {
            resynced: vec!["/World".to_string()],
            changed_info: Vec::new(),
        }],
    };
    let pending = PendingStageChanges {
        batch: Some(batch.clone()),
    };

    assert_eq!(pending.batch(), Some(&batch));
    assert_eq!(pending.batch(), Some(&batch));
}

#[test]
fn plugin_publishes_one_batch_for_later_consumers() {
    let stage = Stage::builder()
        .in_memory("pending-stage-changes.usda")
        .expect("in-memory stage");
    let mut app = App::new();
    app.add_plugins(LiveStagePlugin);
    app.world_mut().insert_non_send(LiveStage::new(stage));

    // The first update performs the initial projection and clears any
    // pre-projection notices, so later notices are the live stream.
    app.update();
    assert!(
        app.world()
            .resource::<PendingStageChanges>()
            .batch()
            .is_none()
    );

    app.world()
        .get_non_send::<LiveStage>()
        .expect("live stage after projection")
        .enqueue_resync("/World");
    app.update();

    let pending = app.world().resource::<PendingStageChanges>();
    let batch = pending.batch().expect("drained batch is published");
    assert_eq!(batch.revision, LiveRevision(1));
    assert_eq!(batch.changes.len(), 1);

    // No second drain means the next empty frame clears the fan-out slot.
    app.update();
    assert!(
        app.world()
            .resource::<PendingStageChanges>()
            .batch()
            .is_none()
    );
}

#[test]
fn project_style_stage_authoring_feeds_one_shared_batch() {
    let stage = Stage::builder()
        .in_memory("project-style-authoring.usda")
        .expect("in-memory stage");
    let live = LiveStage::new(stage);

    // Project/OpenUSD authoring uses the same Stage reference that owns the
    // installed sink; it does not maintain a parallel renderer invalidator.
    crate::authoring::define_prim(&live.stage, "/ProjectRoot", "Xform")
        .expect("author Project root");
    crate::authoring::define_prim(&live.stage, "/ProjectRoot/Model", "Xform")
        .expect("author Project Model");

    let batch = live.drain_change_batch().expect("one Project change batch");
    assert_eq!(batch.revision, LiveRevision(1));
    assert!(batch.has_resync());
    assert!(batch.is_path_under_resync("/ProjectRoot/Model"));

    // These are the same owned batch/revision that projection, semantic sync,
    // and recovery receive after the one authoritative drain.
    let projection_batch = batch.clone();
    let semantic_batch = batch.clone();
    let recovery_batch = batch;
    assert_eq!(projection_batch, semantic_batch);
    assert_eq!(semantic_batch, recovery_batch);
    assert_eq!(live.current_revision(), LiveRevision(1));
    assert!(live.drain_change_batch().is_none());
}
