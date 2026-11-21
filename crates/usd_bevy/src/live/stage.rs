use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use openusd::usd::{CommittedChange, Resynced, Stage, StageSink, StageSinkId};

use super::change::{LiveRevision, StageChange, StageChangeBatch};
use super::visual_stage::VisualStage;

static NEXT_LIVE_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// Identity of a published visual document context, including its projection epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisualContextId {
    session: u64,
    stage_generation: u64,
    authoring_revision: LiveRevision,
    projection_revision: LiveRevision,
}

/// The live, editable USD stage and its change queue. **Non-send** — insert
/// via `world.insert_non_send(LiveStage::new(stage))`.
///
/// Authoring goes through `live.stage` (every method is `&self`); each commit
/// fires the installed sink, which records a [`StageChange`] onto the queue.
/// A reprojection system drains the queue once per frame.
pub struct LiveStage {
    pub stage: Stage,
    visual: RefCell<Option<VisualStage>>,
    session_id: u64,
    stage_generation: u64,
    queue: Rc<RefCell<Vec<StageChange>>>,
    revision: Cell<LiveRevision>,
    visual_queue: RefCell<Vec<StageChange>>,
    visual_revision: Cell<LiveRevision>,
    failed_visual_authoring: Cell<Option<LiveRevision>>,
    authoring_generation: Rc<Cell<LiveRevision>>,
    authoring_frozen: Cell<bool>,
    // Prim paths whose *next* change was caused by our own author-back and
    // should be swallowed once (the echo guard, PLAN P2). Author-back writes
    // the value the component already holds, so a re-project would be a no-op
    // — but skipping it avoids redundant work and any mid-edit churn.
    suppressed: Rc<RefCell<HashSet<String>>>,
    // Kept so the sink lives as long as the stage; removed on drop.
    sink: Option<StageSinkId>,
}

/// Failure-safe reservation for one self-authored prim echo.
///
/// The reservation is retained only when the caller commits the authoring
/// operation. Dropping an uncommitted guard removes the suppression entry so
/// a failed write cannot hide a later real stage change.
pub struct AuthoredSuppressionGuard {
    suppressed: Rc<RefCell<HashSet<String>>>,
    prim: String,
    committed: bool,
}

impl AuthoredSuppressionGuard {
    /// Retain the suppression entry because the guarded authoring succeeded.
    pub fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for AuthoredSuppressionGuard {
    fn drop(&mut self) {
        if !self.committed {
            self.suppressed.borrow_mut().remove(&self.prim);
        }
    }
}

fn push_committed_change(
    queue: &RefCell<Vec<StageChange>>,
    generation: &Cell<LiveRevision>,
    change: &CommittedChange<'_>,
) {
    let next = generation
        .get()
        .0
        .checked_add(1)
        .expect("live stage authoring generation exhausted");
    generation.set(LiveRevision(next));

    let mut resynced = Vec::new();
    let mut changed_info = Vec::new();
    for path in change.resynced {
        if path.is_property() {
            // OpenUSD 0.8 reports newly created/removed property specs in
            // `CommittedChange::resynced`; the owning prim composes unchanged,
            // so route them as property info changes rather than subtree resyncs.
            changed_info.push(path.as_str().to_string());
        } else {
            resynced.push(path.as_str().to_string());
        }
    }
    for path in change.recomposed {
        resynced.push(path.as_str().to_string());
    }
    for path in change.asset_paths_resynced {
        resynced.push(path.as_str().to_string());
    }
    for path in change.changed_info_only {
        changed_info.push(path.as_str().to_string());
    }
    queue.borrow_mut().push(StageChange {
        resynced,
        changed_info,
    });
}

fn push_resynced_notice(queue: &RefCell<Vec<StageChange>>, resynced_notice: Resynced<'_>) {
    let mut resynced =
        Vec::with_capacity(resynced_notice.subtrees.len() + resynced_notice.prims.len());
    for path in resynced_notice.subtrees {
        resynced.push(path.as_str().to_string());
    }
    for path in resynced_notice.prims {
        resynced.push(path.as_str().to_string());
    }
    if !resynced.is_empty() {
        queue.borrow_mut().push(StageChange {
            resynced,
            changed_info: Vec::new(),
        });
    }
}

struct LiveStageSink {
    queue: Rc<RefCell<Vec<StageChange>>>,
    generation: Rc<Cell<LiveRevision>>,
}

impl StageSink for LiveStageSink {
    fn after_commit(&self, _stage: &Stage, change: &CommittedChange<'_>) {
        push_committed_change(&self.queue, &self.generation, change);
    }

    fn layer_muting_changed(
        &self,
        _stage: &Stage,
        _layer: &str,
        _muted: bool,
        resynced: Resynced<'_>,
    ) {
        push_resynced_notice(&self.queue, resynced);
    }

    fn load_rules_changed(&self, _stage: &Stage, resynced: Resynced<'_>) {
        push_resynced_notice(&self.queue, resynced);
    }

    fn loads_retried(&self, _stage: &Stage, resynced: Resynced<'_>) {
        push_resynced_notice(&self.queue, resynced);
    }

    fn recomposed(&self, _stage: &Stage, resynced: Resynced<'_>) {
        push_resynced_notice(&self.queue, resynced);
    }
}

impl LiveStage {
    /// Compose and validate both file-backed contexts before publication.
    pub fn open_document(path: &std::path::Path) -> anyhow::Result<Self> {
        let (stage, visual) = VisualStage::open_pair(path)?;
        let live = Self::new(stage);
        live.visual.replace(Some(visual));
        Ok(live)
    }

    /// Open just the authoritative context for an exact read of a file snapshot,
    /// such as a materialized Git revision.
    pub fn open_authoritative(path: &std::path::Path) -> anyhow::Result<Stage> {
        super::visual_stage::open_read_only(path)
    }

    /// Compose retained source bytes while preserving their resolved USD
    /// locations for relative paths. Composition cannot read other file bytes.
    pub fn open_authoritative_from_source_closure(
        path: &std::path::Path,
        source: impl IntoIterator<Item = (std::path::PathBuf, Vec<u8>)>,
    ) -> anyhow::Result<Stage> {
        super::visual_stage::open_retained(path, source)
    }

    /// The only raw visual-stage access is for engine projection and sampling.
    pub(super) fn projection_stage(&self) -> Stage {
        self.visual
            .borrow()
            .as_ref()
            .map_or_else(|| self.stage.clone(), |visual| visual.stage.clone())
    }

    pub(super) fn install_texture_snapshot(&self, world: &mut bevy::prelude::World) {
        use super::visual_stage::SnapshotTextures;
        let visual = self.visual.borrow();
        let identity = self.stage_identity();
        let current = world.get_non_send::<SnapshotTextures>().map(|source| source.stage_identity);
        if current == visual.as_ref().map(|_| identity) {
            return;
        }
        if let Some(mut cache) = world.get_resource_mut::<crate::route::material::UsdTextureCache>() {
            cache.invalidate_textures();
        }
        if let Some(visual) = visual.as_ref() {
            world.insert_non_send(visual.texture_source(identity));
        } else {
            world.remove_non_send::<SnapshotTextures>();
        }
    }

    fn synchronize_visual(&self) -> anyhow::Result<()> {
        if let Some(visual) = self.visual.borrow_mut().as_mut() {
            visual.synchronize(&self.stage, self.authoring_generation())?;
        }
        Ok(())
    }

    /// Wrap a stage and install the change sink.
    pub fn new(stage: Stage) -> Self {
        let queue: Rc<RefCell<Vec<StageChange>>> = Rc::new(RefCell::new(Vec::new()));
        let authoring_generation = Rc::new(Cell::new(LiveRevision::default()));
        let sink = stage
            .add_sink(LiveStageSink {
                queue: queue.clone(),
                generation: authoring_generation.clone(),
            })
            .expect("stage sink registration failed");
        Self {
            stage,
            visual: RefCell::default(),
            session_id: NEXT_LIVE_SESSION_ID.fetch_add(1, Ordering::Relaxed),
            stage_generation: 0,
            queue,
            revision: Cell::new(LiveRevision::default()),
            visual_queue: RefCell::default(),
            visual_revision: Cell::default(),
            failed_visual_authoring: Cell::default(),
            authoring_generation,
            authoring_frozen: Cell::new(false),
            suppressed: Rc::new(RefCell::new(HashSet::new())),
            sink: Some(sink),
        }
    }

    /// Replace the owned USD stage with a freshly synchronized canonical
    /// snapshot. The LiveStage session remains the same service lifetime, but
    /// the stage generation changes so stage-scoped consumers can invalidate
    /// derived state before the root resync is projected.
    pub fn replace_stage(&mut self, stage: Stage) {
        self.replace_stage_and_visual(stage, None);
    }

    /// Recompose the same document session from a new validated file snapshot.
    pub fn replace_document(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        let (stage, visual) = VisualStage::open_pair(path)?;
        self.replace_stage_and_visual(stage, Some(visual));
        Ok(())
    }

    fn replace_stage_and_visual(&mut self, stage: Stage, mut visual: Option<VisualStage>) {
        if let Some(id) = self.sink.take() {
            let _ = self.stage.remove_sink(id);
        }
        let queue: Rc<RefCell<Vec<StageChange>>> = Rc::new(RefCell::new(Vec::new()));
        let authoring_generation = Rc::new(Cell::new(LiveRevision(
            self.authoring_generation
                .get()
                .0
                .checked_add(1)
                .expect("live stage authoring generation exhausted"),
        )));
        if let Some(visual) = visual.as_mut() {
            visual.authoring_revision = authoring_generation.get();
        }
        let sink = stage
            .add_sink(LiveStageSink {
                queue: queue.clone(),
                generation: authoring_generation.clone(),
            })
            .expect("stage sink registration failed");
        self.stage = stage;
        self.visual.replace(visual);
        self.stage_generation = self
            .stage_generation
            .checked_add(1)
            .expect("live stage generation exhausted");
        self.queue = queue;
        self.visual_queue.get_mut().clear();
        self.failed_visual_authoring.set(None);
        self.authoring_generation = authoring_generation;
        self.suppressed = Rc::new(RefCell::new(HashSet::new()));
        self.sink = Some(sink);
        self.enqueue_resync("/");
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
        if self.visual.borrow().is_some() {
            self.visual_queue
                .borrow_mut()
                .extend(changes.iter().cloned());
        }
        Some(StageChangeBatch { revision, changes })
    }

    pub(super) fn drain_batches(&self) -> (Option<StageChangeBatch>, Option<StageChangeBatch>) {
        let canonical = self.drain_change_batch();
        if self.visual.borrow().is_none() {
            return (canonical.clone(), canonical);
        }
        let authoring = self.authoring_generation();
        if self.visual_queue.borrow().is_empty()
            || self.failed_visual_authoring.get() == Some(authoring)
        {
            return (canonical, None);
        }
        if let Err(error) = self.synchronize_visual() {
            self.failed_visual_authoring.set(Some(authoring));
            bevy::log::error!("could not synchronize the visual USD revision: {error:#}");
            return (canonical, None);
        }
        let revision = LiveRevision(
            self.visual_revision
                .get()
                .0
                .checked_add(1)
                .expect("visual stage revision exhausted"),
        );
        self.visual_revision.set(revision);
        let changes = std::mem::take(&mut *self.visual_queue.borrow_mut());
        (canonical, Some(StageChangeBatch { revision, changes }))
    }

    /// The most recently drained live revision.
    pub fn current_revision(&self) -> LiveRevision {
        self.revision.get()
    }

    /// Synchronous authoring token. Unlike [`current_revision`], this changes
    /// in the OpenUSD commit callback before the next frame drains notices.
    pub fn authoring_generation(&self) -> LiveRevision {
        self.authoring_generation.get()
    }

    /// Acquire the exclusive authoring lease used by Project Commit/Export.
    pub fn try_freeze_authoring(&self) -> bool {
        if self.authoring_frozen.replace(true) {
            return false;
        }
        true
    }

    /// Release the exclusive authoring lease.
    pub fn unfreeze_authoring(&self) {
        self.authoring_frozen.set(false);
    }

    pub fn is_authoring_frozen(&self) -> bool {
        self.authoring_frozen.get()
    }

    /// Stable identity for this live-stage lifetime, distinct across reloads.
    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    /// Identity of the current authoritative composed Stage.
    pub fn stage_identity(&self) -> (u64, u64) {
        (self.session_id, self.stage_generation)
    }

    pub fn visual_context_id(&self) -> VisualContextId {
        let visual = self.visual.borrow();
        let (authoring_revision, projection_revision) = visual.as_ref().map_or_else(
            || (self.authoring_generation(), self.current_revision()),
            |visual| (visual.authoring_revision, self.visual_revision.get()),
        );
        VisualContextId {
            session: self.session_id,
            stage_generation: self.stage_generation,
            authoring_revision,
            projection_revision,
        }
    }

    /// Whether any change is pending (cheap check before doing work).
    pub fn has_changes(&self) -> bool {
        !self.queue.borrow().is_empty() || !self.visual_queue.borrow().is_empty()
    }

    /// Mark `prim` as self-authored: the next change mentioning it (fired by
    /// our own author-back) is swallowed by [`super::reconcile::apply_changes`] rather than
    /// re-projected. Call immediately before authoring.
    pub fn mark_authored(&self, prim: impl Into<String>) {
        self.suppressed.borrow_mut().insert(prim.into());
    }

    /// Reserve a self-authored suppression entry with failure-safe cleanup.
    pub fn mark_authored_guard(&self, prim: impl Into<String>) -> AuthoredSuppressionGuard {
        let prim = prim.into();
        self.mark_authored(prim.clone());
        AuthoredSuppressionGuard {
            suppressed: Rc::clone(&self.suppressed),
            prim,
            committed: false,
        }
    }

    /// Run one authoring operation with failure-safe self-authored tracking.
    pub fn with_authored<T, E>(
        &self,
        prim: impl Into<String>,
        author: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        let suppression = self.mark_authored_guard(prim);
        let result = author();
        if result.is_ok() {
            suppression.commit();
        }
        result
    }

    /// Take and clear the set of self-authored prim paths.
    pub(super) fn take_suppressed(&self) -> HashSet<String> {
        std::mem::take(&mut *self.suppressed.borrow_mut())
    }

    /// Load `prim`'s payload (and everything beneath it). This is a composition
    /// change — it fires the change sink, so the next `apply_changes` reconciles
    /// and the newly-composed subtree is projected. The reversible counterpart
    /// of BSN's `queue_spawn_scene`.
    pub fn load_payload(&self, prim: &str) -> anyhow::Result<()> {
        self.synchronize_visual()?;
        let p = openusd::sdf::path(prim)?;
        self.projection_stage()
            .load(p, openusd::usd::LoadPolicy::WithDescendants)?;
        self.enqueue_visual_resync(prim);
        Ok(())
    }

    /// Unload `prim`'s payload — the projected subtree is despawned on the next
    /// `apply_changes` and the prim is marked
    /// [`UsdPayloadUnloaded`](crate::route::payload::UsdPayloadUnloaded).
    pub fn unload_payload(&self, prim: &str) -> anyhow::Result<()> {
        self.synchronize_visual()?;
        let p = openusd::sdf::path(prim)?;
        self.projection_stage().unload(p)?;
        self.enqueue_visual_resync(prim);
        Ok(())
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

    fn enqueue_visual_resync(&self, prim: &str) {
        if self.visual.borrow().is_some() {
            self.visual_queue.borrow_mut().push(StageChange {
                resynced: vec![prim.to_owned()],
                changed_info: Vec::new(),
            });
        } else {
            self.enqueue_resync(prim);
        }
    }
}

impl Drop for LiveStage {
    fn drop(&mut self) {
        if let Some(id) = self.sink.take() {
            let _ = self.stage.remove_sink(id);
        }
    }
}
