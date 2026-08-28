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
fn failed_authoring_drops_self_authored_suppression() {
    let stage = Stage::builder()
        .in_memory("failed-authoring-suppression.usda")
        .expect("in-memory stage");
    let live = LiveStage::new(stage);
    let suppression = live.mark_authored_guard("/World/A");

    let result = crate::authoring::move_prim(&live.stage, "/World/Missing", "/World/A");
    assert!(result.is_err());
    drop(suppression);

    assert!(live.take_suppressed().is_empty());
}

#[test]
fn committed_authoring_keeps_one_self_authored_suppression() {
    let stage = Stage::builder()
        .in_memory("committed-authoring-suppression.usda")
        .expect("in-memory stage");
    let live = LiveStage::new(stage);
    let suppression = live.mark_authored_guard("/World/A");
    suppression.commit();

    assert_eq!(live.take_suppressed().len(), 1);
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
