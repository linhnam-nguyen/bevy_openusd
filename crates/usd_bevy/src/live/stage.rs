use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use openusd::usd::{CommittedChange, Stage, StageSinkId};

use super::change::{LiveRevision, StageChange, StageChangeBatch};

static NEXT_LIVE_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// The live, editable USD stage and its change queue. **Non-send** — insert
/// via `world.insert_non_send(LiveStage::new(stage))`.
///
/// Authoring goes through `live.stage` (every method is `&self`); each commit
/// fires the installed sink, which records a [`StageChange`] onto the queue.
/// A reprojection system drains the queue once per frame.
pub struct LiveStage {
    pub stage: Stage,
    session_id: u64,
    queue: Rc<RefCell<Vec<StageChange>>>,
    revision: Cell<LiveRevision>,
    // Prim paths whose *next* change was caused by our own author-back and
    // should be swallowed once (the echo guard, PLAN P2). Author-back writes
    // the value the component already holds, so a re-project would be a no-op
    // — but skipping it avoids redundant work and any mid-edit churn.
    suppressed: Rc<RefCell<HashSet<String>>>,
    // Kept so the sink lives as long as the stage; removed on drop.
    sink: Option<StageSinkId>,
}

impl LiveStage {
    /// Wrap a stage and install the change sink.
    pub fn new(stage: Stage) -> Self {
        let queue: Rc<RefCell<Vec<StageChange>>> = Rc::new(RefCell::new(Vec::new()));
        let q = queue.clone();
        let sink = stage.add_sink(move |_stage: &Stage, change: &CommittedChange<'_>| {
            q.borrow_mut().push(StageChange {
                resynced: change
                    .resynced
                    .iter()
                    .map(|p| p.as_str().to_string())
                    .collect(),
                changed_info: change
                    .changed_info_only
                    .iter()
                    .map(|p| p.as_str().to_string())
                    .collect(),
            });
        });
        Self {
            stage,
            session_id: NEXT_LIVE_SESSION_ID.fetch_add(1, Ordering::Relaxed),
            queue,
            revision: Cell::new(LiveRevision::default()),
            suppressed: Rc::new(RefCell::new(HashSet::new())),
            sink: Some(sink),
        }
    }

    /// Take and clear all changes recorded since the last drain.
    ///
    /// A non-empty drain advances the live revision exactly once. Callers
    /// should pass the returned batch to every consumer for the frame rather
    /// than draining the stage again.
    pub fn drain_change_batch(&self) -> Option<StageChangeBatch> {
        let changes = std::mem::take(&mut *self.queue.borrow_mut());
        if changes.is_empty() {
            return None;
        }
        let revision = LiveRevision(
            self.revision
                .get()
                .0
                .checked_add(1)
                .expect("live stage revision exhausted"),
        );
        self.revision.set(revision);
        Some(StageChangeBatch { revision, changes })
    }

    /// The most recently drained live revision.
    pub fn current_revision(&self) -> LiveRevision {
        self.revision.get()
    }

    /// Stable identity for this live-stage lifetime, distinct across reloads.
    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    /// Whether any change is pending (cheap check before doing work).
    pub fn has_changes(&self) -> bool {
        !self.queue.borrow().is_empty()
    }

    /// Mark `prim` as self-authored: the next change mentioning it (fired by
    /// our own author-back) is swallowed by [`super::reconcile::apply_changes`] rather than
    /// re-projected. Call immediately before authoring.
    pub fn mark_authored(&self, prim: impl Into<String>) {
        self.suppressed.borrow_mut().insert(prim.into());
    }

    /// Take and clear the set of self-authored prim paths.
    pub(super) fn take_suppressed(&self) -> HashSet<String> {
        std::mem::take(&mut *self.suppressed.borrow_mut())
    }

    /// Load `prim`'s payload (and everything beneath it). This is a composition
    /// change — it fires the change sink, so the next `apply_changes` reconciles
    /// and the newly-composed subtree is projected. The reversible counterpart
    /// of BSN's `queue_spawn_scene`.
    pub fn load_payload(&self, prim: &str) {
        if let Ok(p) = openusd::sdf::path(prim) {
            self.stage
                .load(p, openusd::usd::LoadPolicy::WithDescendants);
            self.enqueue_resync(prim);
        }
    }

    /// Unload `prim`'s payload — the projected subtree is despawned on the next
    /// `apply_changes` and the prim is marked
    /// [`UsdPayloadUnloaded`](crate::route::payload::UsdPayloadUnloaded).
    pub fn unload_payload(&self, prim: &str) {
        if let Ok(p) = openusd::sdf::path(prim) {
            self.stage.unload(p);
            self.enqueue_resync(prim);
        }
    }

    /// Enqueue a `resynced` change for `prim`. openusd's `load`/`unload` change
    /// composition but do **not** fire the authoring change sink (they are
    /// stage load-rule changes, not layer-edit commits), so we synthesize the
    /// notice ourselves — the reconcile then materializes/despawns the subtree.
    pub fn enqueue_resync(&self, prim: &str) {
        self.queue.borrow_mut().push(StageChange {
            resynced: vec![prim.to_string()],
            changed_info: Vec::new(),
        });
    }
}

impl Drop for LiveStage {
    fn drop(&mut self) {
        if let Some(id) = self.sink.take() {
            self.stage.remove_sink(id);
        }
    }
}
