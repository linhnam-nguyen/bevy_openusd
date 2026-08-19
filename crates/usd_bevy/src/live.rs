//! Live, editable USD stage + change-driven reprojection (RETHINK P2/P3).
//!
//! The composed USD stage is the single source of truth. We hold it live
//! (not baked to a one-shot `Scene`), project it into Bevy entities, and
//! keep them in sync off openusd's `StageSink` (`UsdNotice`) change stream:
//! every committed edit fires the sink, we copy the changed paths out, and a
//! Bevy system reprojects exactly the affected entities.
//!
//! The openusd `Stage` is `Rc`/`RefCell`-backed (`!Send`), so [`LiveStage`]
//! is a **non-send** resource (main thread only). The path↔entity index
//! [`PrimEntities`] is plain data and is a normal `Resource`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use bevy::prelude::*;
use openusd::usd::{CommittedChange, Stage, StageSinkId};

static NEXT_LIVE_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// One committed stage change, copied out of the borrowed [`CommittedChange`]
/// so it can outlive the sink callback and be drained on a later frame.
///
/// * `resynced` — composition restructured (define / remove / reparent /
///   variant / reference / layer-mute …); the subtree must be reprojected.
/// * `changed_info` — a field/value/target changed, namespace intact; the
///   corresponding component(s) can be patched in place.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StageChange {
    pub resynced: Vec<String>,
    pub changed_info: Vec<String>,
}

/// Monotonic revision of the in-memory live stage.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LiveRevision(pub u64);

/// One authoritative, once-drained batch of stage changes.
///
/// The batch is retained in [`PendingStageChanges`] for the rest of the
/// frame, so projection, semantic indexing, and diagnostics can all consume
/// the same revision without independently draining [`LiveStage`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageChangeBatch {
    pub revision: LiveRevision,
    pub changes: Vec<StageChange>,
}

/// Normalizes a USD prim or property path to its owning prim path without trailing slashes.
///
/// Leading `/` is ensured, property specifiers (`.property_name`) are stripped defensively,
/// and trailing slashes are removed unless the path is the root `"/"`.
pub fn normalize_prim_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_string();
    }
    let without_prop = match trimmed.split_once('.') {
        Some((prim, _prop)) => prim,
        None => trimmed,
    };
    let mut normalized = without_prop.to_string();
    if !normalized.starts_with('/') {
        normalized.insert(0, '/');
    }
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    if normalized.is_empty() {
        "/".to_string()
    } else {
        normalized
    }
}

/// Validates that a path string can safely represent a normalized OpenUSD prim path.
///
/// Returns the normalized path if valid, or an error if the path contains invalid syntax,
/// unresolvable relative components, or cannot be parsed by OpenUSD.
pub fn validate_prim_path(path: &str) -> anyhow::Result<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return Ok("/".to_string());
    }
    if trimmed.contains("//") || trimmed.contains("..") || trimmed.split('/').any(|seg| seg == ".") {
        anyhow::bail!("path '{path}' contains unsafe relative or empty segments");
    }
    let normalized = normalize_prim_path(path);
    if normalized == "/" {
        return Ok(normalized);
    }
    openusd::sdf::path(&normalized)
        .map_err(|e| anyhow::anyhow!("invalid OpenUSD prim path '{normalized}': {e:#}"))?;
    Ok(normalized)
}

/// Checks whether `candidate` is equal to or a descendant of `ancestor` with boundary awareness.
///
/// This prevents naive substring matches like `/World/A` falsely matching `/World/AB`.
pub fn is_descendant_or_self(ancestor: &str, candidate: &str) -> bool {
    let ancestor = normalize_prim_path(ancestor);
    let candidate = normalize_prim_path(candidate);

    if ancestor == "/" {
        return true;
    }
    if ancestor == candidate {
        return true;
    }
    if candidate.starts_with(&ancestor) {
        let after_ancestor = &candidate[ancestor.len()..];
        return after_ancestor.starts_with('/') || after_ancestor.starts_with('.');
    }
    false
}

/// Normalizes and minimizes a set of resync candidate paths.
///
/// Deduplicates exact duplicates, sorts shallowest first, and prunes any child path
/// whose ancestor is already included. If the stage root `"/"` is present, returns `["/"]`.
pub fn minimize_resync_roots<I, S>(paths: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut normalized_set = std::collections::HashSet::new();
    for p in paths {
        let norm = normalize_prim_path(p.as_ref());
        if norm == "/" {
            return vec!["/".to_string()];
        }
        normalized_set.insert(norm);
    }

    if normalized_set.is_empty() {
        return Vec::new();
    }

    let mut sorted: Vec<String> = normalized_set.into_iter().collect();
    // Sort primarily by segment depth (fewer '/' means shallower root), secondarily lexicographically
    sorted.sort_by(|a, b| {
        let depth_a = a.matches('/').count();
        let depth_b = b.matches('/').count();
        depth_a.cmp(&depth_b).then_with(|| a.cmp(b))
    });

    let mut accepted: Vec<String> = Vec::new();
    for candidate in sorted {
        let is_covered = accepted
            .iter()
            .any(|root| is_descendant_or_self(root, &candidate));
        if !is_covered {
            accepted.push(candidate);
        }
    }
    accepted
}

impl StageChangeBatch {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Returns `true` if any change in the batch contains a resync notice.
    pub fn has_resync(&self) -> bool {
        self.changes.iter().any(|c| !c.resynced.is_empty())
    }

    /// Returns the minimal, boundary-aware resync roots that cover all resynced paths
    /// in this batch.
    pub fn resync_roots(&self) -> Vec<String> {
        let all_resynced = self.changes.iter().flat_map(|c| &c.resynced);
        minimize_resync_roots(all_resynced)
    }

    /// Checks if a given path (prim or property) falls under any resync root in this batch.
    pub fn is_path_under_resync(&self, path: &str) -> bool {
        let roots = self.resync_roots();
        roots.iter().any(|root| is_descendant_or_self(root, path))
    }

    /// Returns all `changed_info` paths from this batch that are outside all resync roots.
    ///
    /// Changes under a resync root are owned by subtree reconciliation and should not be
    /// redundantly sparse-patched.
    pub fn unshaded_changed_info(&self) -> Vec<String> {
        let roots = self.resync_roots();
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();

        for change in &self.changes {
            for info_path in &change.changed_info {
                let prim_path = normalize_prim_path(info_path);
                let covered = roots
                    .iter()
                    .any(|root| is_descendant_or_self(root, &prim_path));
                if !covered && seen.insert(info_path.clone()) {
                    result.push(info_path.clone());
                }
            }
        }
        result
    }
}

/// Internal work counters for the most recent reconcile pass.
///
/// Test and profiling suites use this to verify work reduction during subtree
/// resync without relying on noisy timing assertions.
#[derive(Resource, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ReconcileStats {
    pub(crate) roots: usize,
    pub(crate) visited_stage_prims: usize,
    pub(crate) patched_entities: usize,
    pub(crate) spawned_entities: usize,
    pub(crate) despawned_entities: usize,
}

impl StageChange {
    /// All paths mentioned by this change (resynced ∪ changed-info).
    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.resynced.iter().chain(self.changed_info.iter())
    }
}

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
    suppressed: Rc<RefCell<std::collections::HashSet<String>>>,
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
            suppressed: Rc::new(RefCell::new(std::collections::HashSet::new())),
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
    /// our own author-back) is swallowed by [`apply_changes`] rather than
    /// re-projected. Call immediately before authoring.
    pub fn mark_authored(&self, prim: impl Into<String>) {
        self.suppressed.borrow_mut().insert(prim.into());
    }

    /// Take and clear the set of self-authored prim paths.
    fn take_suppressed(&self) -> std::collections::HashSet<String> {
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

/// Bidirectional `SdfPath ↔ Entity` index — the reprojection key. Plain
/// `Resource` (the paths are owned `String`s, the entities are ids).
#[derive(Resource, Default)]
pub struct PrimEntities {
    by_path: HashMap<String, Entity>,
    by_entity: HashMap<Entity, String>,
}

/// The stage-change batch drained for the current frame.
///
/// This is intentionally a transient fan-out resource, not another model
/// representation. It is replaced by [`drain_stage_changes_system`] before
/// each projection pass and remains readable by later consumers in the same
/// schedule.
#[derive(Resource, Default)]
pub struct PendingStageChanges {
    batch: Option<StageChangeBatch>,
}

impl PendingStageChanges {
    pub fn batch(&self) -> Option<&StageChangeBatch> {
        self.batch.as_ref()
    }
}

impl PrimEntities {
    pub fn insert(&mut self, path: impl Into<String>, entity: Entity) {
        let path = path.into();
        self.by_entity.insert(entity, path.clone());
        self.by_path.insert(path, entity);
    }

    pub fn entity(&self, path: &str) -> Option<Entity> {
        self.by_path.get(path).copied()
    }

    pub fn path(&self, entity: Entity) -> Option<&str> {
        self.by_entity.get(&entity).map(String::as_str)
    }

    /// Remove a path's mapping, returning the entity it pointed at.
    pub fn remove_path(&mut self, path: &str) -> Option<Entity> {
        let e = self.by_path.remove(path)?;
        self.by_entity.remove(&e);
        Some(e)
    }

    /// Remove an entity's mapping (e.g. on despawn).
    pub fn remove_entity(&mut self, entity: Entity) -> Option<String> {
        let p = self.by_entity.remove(&entity)?;
        self.by_path.remove(&p);
        Some(p)
    }

    /// Every `(path, entity)` currently mapped.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Entity)> {
        self.by_path.iter().map(|(p, e)| (p.as_str(), *e))
    }

    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    /// Every `(path, entity)` whose path is `prefix` or a descendant of it —
    /// the set a `resynced` parent invalidates.
    pub fn subtree(&self, prefix: &str) -> Vec<(String, Entity)> {
        let norm = normalize_prim_path(prefix);
        self.by_path
            .iter()
            .filter(|(p, _)| is_descendant_or_self(&norm, p.as_str()))
            .map(|(p, e)| (p.clone(), *e))
            .collect()
    }
}

// ─── Projection + reprojection (v1: transforms) ─────────────────────
//
// Minimal slice of the project/sync loop: one entity per prim carrying
// `UsdPrimRef` + `Transform`. Mesh / material / the full field→component
// routing (RETHINK §12) layer on top of this same shape.

use crate::prim_ref::{SemanticEntityIndex, UsdPrimRef};
use crate::read::xform::read_transform;
use crate::route::{SchemaRegistry, StageTime};

/// Prim paths that have at least one time-sampled (animated) attribute — the
/// set the animation resampler revisits when [`StageTime`] changes. Computed
/// once at projection.
#[derive(Resource, Default, Clone)]
pub struct AnimatedPrims(pub std::collections::HashSet<String>);

/// The [`DisplayPurposes`] the projected entities were last filtered against,
/// so the purpose reprojector only reruns when the toggle actually changes.
#[derive(Resource, Default)]
struct AppliedPurposes(Option<crate::route::DisplayPurposes>);

/// The [`StageTime`] the projected entities were last sampled at, so the
/// resampler only reruns when the time actually moves.
#[derive(Resource, Default)]
struct SampledTime(Option<f64>);

/// Whether `prim` animates: it has a time-sampled attribute of its own, or it
/// is a skinned mesh driven by a time-varying SkelAnimation (whose samples live
/// on a different prim).
fn prim_is_animated(stage: &Stage, path: &openusd::sdf::Path) -> bool {
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

fn to_bevy_transform(t: crate::read::xform::Transform3) -> Transform {
    Transform {
        translation: Vec3::from_array(t.translate),
        rotation: Quat::from_array(t.rotate),
        scale: Vec3::from_array(t.scale),
    }
}

/// Rotation mapping the stage's authored up-axis onto Bevy's Y-up world. USD
/// defaults to Y-up; Z-up content (common for robotics / CAD assets) is rotated
/// -90° about X so +Z becomes +Y. Applied once on the stage-root entity so the
/// whole composed scene stands upright on the ground grid.
fn stage_up_axis(stage: &Stage) -> Quat {
    let is_z = matches!(
        stage.stage_metadata("upAxis").ok().flatten(),
        Some(openusd::sdf::Value::Token(t)) if t.as_str() == "Z"
    );
    if is_z {
        Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2)
    } else {
        Quat::IDENTITY
    }
}

/// The namespace parent of a prim path — the pseudo-root `/` for a top-level
/// prim, so it parents onto the stage-root entity.
fn parent_path(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i],
    }
}

/// The prim path owning a (possibly property) path: `/Foo.bar` → `/Foo`.
pub fn prim_of(path: &str) -> &str {
    path.split('.').next().unwrap_or(path)
}

/// The property part of a (possibly property) path: `/Foo.xformOp:x` →
/// `Some("xformOp:x")`; a bare prim path → `None`.
pub fn property_of(path: &str) -> Option<&str> {
    path.split_once('.').map(|(_, prop)| prop)
}

/// The traversal predicate for projection: active + defined + non-abstract, but
/// **not** requiring `LOADED` (unlike `PrimPredicate::default()`). This projects
/// a prim whose payload is *unloaded* as a placeholder (its payloaded children
/// stay absent until [`LiveStage::load_payload`]). For fully-loaded stages this
/// is identical to the default predicate.
fn traverse_predicate() -> openusd::usd::PrimPredicate {
    use openusd::usd::PrimStatus;
    openusd::usd::PrimPredicate::new(
        PrimStatus::ACTIVE.union(PrimStatus::DEFINED),
        PrimStatus::ABSTRACT,
    )
}

/// Collects all valid, projected prim paths within a subtree rooted at `root`.
///
/// Uses the canonical projection predicate (`ACTIVE | DEFINED & ~ABSTRACT`) so
/// that subtree reconciliation and semantic extraction see exactly the prims
/// that the renderer projects.
///
/// # Implementation & Complexity Notes
/// - **Mechanism**: Current helper executes a full-stage traversal (`stage.traverse(...)`)
///   combined with a boundary-aware ancestry filter ([`is_descendant_or_self`]).
/// - **Complexity**: `O(total projected prims)` (full-stage traversal).
///   Subtree resync optimizes downstream work (entity patching/spawning/despawning,
///   semantic extraction, and database row updates), while OpenUSD traversal itself
///   operates across the stage.
///
/// Returns:
/// - `root` and all projected descendants in pre-order if `root` exists.
/// - An empty `Vec` if `root` does not exist on the stage (indicating removal).
/// - All projected stage prims if `root == "/"`.
pub fn collect_stage_subtree_paths(stage: &Stage, root: &str) -> anyhow::Result<Vec<String>> {
    let normalized_root = validate_prim_path(root)?;
    let mut collected = Vec::new();
    stage.traverse(traverse_predicate(), |path: &openusd::sdf::Path| {
        let path_str = path.as_str();
        if is_descendant_or_self(&normalized_root, path_str) {
            collected.push(path_str.to_string());
        }
    })?;
    Ok(collected)
}

/// Snapshot the registry out of the world (Arc-cheap `Clone`), falling back to
/// the built-in routes when none is installed — so direct `project_stage` /
/// `apply_changes` calls in tests work without wiring a registry.
fn registry_of(world: &World) -> SchemaRegistry {
    world
        .get_resource::<SchemaRegistry>()
        .cloned()
        .unwrap_or_else(SchemaRegistry::builtin)
}

/// Project every prim in the stage into an entity (`UsdPrimRef` +
/// `Transform`), recording the path↔entity bimap. Idempotent only on an
/// empty world — call once on load.
pub fn project_stage(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let stage = &live.stage;
    let root = world
        .spawn((
            UsdPrimRef {
                path: "/".to_string(),
            },
            Transform::from_rotation(stage_up_axis(stage)),
            Visibility::default(),
        ))
        .id();
    map.insert("/", root);

    let registry = registry_of(world);
    let mut ordered: Vec<String> = Vec::new();
    let _ = stage.traverse(traverse_predicate(), |p: &openusd::sdf::Path| {
        ordered.push(p.as_str().to_string());
    });
    ordered.sort_by_key(|p| p.matches('/').count());

    let mut animated = std::collections::HashSet::new();
    for path in ordered {
        let Ok(p) = openusd::sdf::path(&path) else {
            continue;
        };
        if prim_is_animated(stage, &p) {
            animated.insert(path.clone());
        }
        let parent = map.entity(parent_path(&path)).unwrap_or(root);
        let mut e = world.spawn(UsdPrimRef { path: path.clone() });
        e.insert(ChildOf(parent));
        let entity = e.id();
        map.insert(path.clone(), entity);
        registry.project_prim(stage, &p, world, entity);
    }
    if let Some(mut animated_res) = world.get_resource_mut::<AnimatedPrims>() {
        animated_res.0 = animated;
    }
}

/// Drain the change queue and reproject affected entities.
///
/// * Any `resynced` change → reconcile the entity set against the stage
///   (spawn entities for new prims, despawn entities for removed prims,
///   patch the rest). v1 reconciles the whole stage; a later version scopes
///   to the resynced subtree.
/// * `changed_info` only → patch the touched prims' transforms in place.
pub fn apply_changes(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let Some(batch) = live.drain_change_batch() else {
        return;
    };
    apply_change_batch(world, live, map, &batch);
}

/// Sparse property patch applied per owning prim. Each prim's registered route
/// runs with `changed` pointing to only the properties modified in the batch.
fn apply_sparse_changed_info(
    world: &mut World,
    live: &LiveStage,
    map: &mut PrimEntities,
    changed_info: &[String],
) {
    if changed_info.is_empty() {
        return;
    }
    let registry = registry_of(world);
    let suppressed = live.take_suppressed();
    let mut per_prim: HashMap<String, Vec<String>> = HashMap::new();
    for prop_path in changed_info {
        let prim = prim_of(prop_path);
        let prop = property_of(prop_path).unwrap_or("");
        per_prim
            .entry(prim.to_string())
            .or_default()
            .push(prop.to_string());
    }

    for (prim, props) in per_prim {
        if suppressed.contains(&prim) {
            continue;
        }
        let Ok(p) = openusd::sdf::path(&prim) else {
            continue;
        };
        let Some(entity) = map.entity(&prim) else {
            continue;
        };
        let prop_refs: Vec<&str> = props.iter().map(String::as_str).collect();
        registry.patch_prim(&live.stage, &p, world, entity, &prop_refs);
    }
}

/// Reproject one already-drained batch without touching the live-stage queue.
pub fn apply_change_batch(
    world: &mut World,
    live: &LiveStage,
    map: &mut PrimEntities,
    batch: &StageChangeBatch,
) {
    if batch.is_empty() {
        return;
    }
    if batch.has_resync() {
        let roots = batch.resync_roots();
        if roots.contains(&"/".to_string()) || roots.is_empty() {
            bevy::log::warn!(
                target: "usd_bevy",
                resync_fallback_reason = "root_is_stage_root_or_empty",
                root_count = roots.len(),
                live_revision = batch.revision.0,
                "[subtree-reconcile] stage root '/' or empty roots in batch; falling back to full reconcile"
            );
            reconcile_full(world, live, map);
        } else {
            // Explicit resync-root validation after normalization
            let mut validated_roots = Vec::with_capacity(roots.len());
            let mut unnormalizable = false;
            for r in &roots {
                match validate_prim_path(r) {
                    Ok(val) => validated_roots.push(val),
                    Err(err) => {
                        bevy::log::warn!(
                            target: "usd_bevy",
                            resync_fallback_reason = "unnormalizable_root",
                            root_count = roots.len(),
                            live_revision = batch.revision.0,
                            "[subtree-reconcile] root '{r}' cannot represent a safe OpenUSD prim path: {err:#}; falling back to full reconcile"
                        );
                        unnormalizable = true;
                        break;
                    }
                }
            }
            if unnormalizable {
                reconcile_full(world, live, map);
            } else {
                reconcile_subtrees(world, live, map, &validated_roots, batch.revision);
            }
        }
        let unshaded = batch.unshaded_changed_info();
        apply_sparse_changed_info(world, live, map, &unshaded);
        return;
    }

    // `changed_info` only: group all changed property paths by owning prim and patch sparsely.
    let all_changed_info: Vec<String> = batch
        .changes
        .iter()
        .flat_map(|c| &c.changed_info)
        .cloned()
        .collect();
    apply_sparse_changed_info(world, live, map, &all_changed_info);
}

/// Reconcile specific subtrees against the stage's current prims.
fn reconcile_subtrees(
    world: &mut World,
    live: &LiveStage,
    map: &mut PrimEntities,
    roots: &[String],
    revision: LiveRevision,
) {
    let stage = &live.stage;
    let registry = registry_of(world);
    let Some(root_entity) = map.entity("/") else {
        bevy::log::warn!(
            target: "usd_bevy",
            resync_fallback_reason = "root_entity_missing",
            root_count = roots.len(),
            live_revision = revision.0,
            "[subtree-reconcile] stage root '/' missing from PrimEntities; falling back to full reconcile"
        );
        reconcile_full(world, live, map);
        return;
    };

    let mut old_entities: HashMap<String, Entity> = HashMap::new();
    for root in roots {
        for (path, entity) in map.subtree(root) {
            if path != "/" {
                old_entities.insert(path, entity);
            }
        }
    }

    let mut current_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
    for root in roots {
        match collect_stage_subtree_paths(stage, root) {
            Ok(paths) => {
                current_paths.extend(paths);
            }
            Err(error) => {
                bevy::log::warn!(
                    target: "usd_bevy",
                    resync_fallback_reason = "subtree_collection_failed",
                    root_count = roots.len(),
                    live_revision = revision.0,
                    "[subtree-reconcile] collection failed for root '{root}': {error:#}; falling back to full reconcile"
                );
                reconcile_full(world, live, map);
                return;
            }
        }
    }

    // 1. Preflight parent integrity for all new prims (current_paths - old_paths) BEFORE any mutations
    let mut added: Vec<String> = current_paths
        .iter()
        .filter(|path| !old_entities.contains_key(*path))
        .cloned()
        .collect();
    added.sort_by(|a, b| a.matches('/').count().cmp(&b.matches('/').count()));

    for path in &added {
        let parent_str = parent_path(path);
        let parent_will_exist = if parent_str == "/" {
            true
        } else {
            current_paths.contains(parent_str) || map.entity(parent_str).is_some()
        };

        if !parent_will_exist {
            bevy::log::warn!(
                target: "usd_bevy",
                resync_fallback_reason = "unresolved_parent",
                root_count = roots.len(),
                live_revision = revision.0,
                "[subtree-reconcile] parent '{parent_str}' for new prim '{path}' is missing; falling back to full reconcile"
            );
            reconcile_full(world, live, map);
            return;
        }
    }

    // 2. Despawn removed prims (old_paths - current_paths), deepest first
    let mut removed: Vec<(String, Entity)> = old_entities
        .iter()
        .filter(|(path, _)| !current_paths.contains(*path))
        .map(|(path, entity)| (path.clone(), *entity))
        .collect();
    removed.sort_by(|(a, _), (b, _)| b.matches('/').count().cmp(&a.matches('/').count()));

    let despawned_count = removed.len();
    for (path, entity) in removed {
        if let Some(mut semantic_idx) = world.get_resource_mut::<SemanticEntityIndex>() {
            semantic_idx.remove_entity(entity);
        }
        world.despawn(entity);
        map.remove_path(&path);
    }

    // 3. Spawn new prims (current_paths - old_paths), shallowest first
    let mut spawned_count = 0usize;
    for path in &added {
        let Ok(p) = openusd::sdf::path(path) else {
            continue;
        };
        let parent_str = parent_path(path);
        let parent = if parent_str == "/" {
            Some(root_entity)
        } else {
            map.entity(parent_str)
        };
        let Some(parent) = parent else {
            bevy::log::error!(
                target: "usd_bevy",
                resync_fallback_reason = "missing_parent_entity",
                root_count = roots.len(),
                live_revision = revision.0,
                "[subtree-reconcile] parent '{parent_str}' missing during spawn for '{path}'; falling back to full reconcile"
            );
            reconcile_full(world, live, map);
            return;
        };
        let mut e = world.spawn(UsdPrimRef { path: path.clone() });
        e.insert(ChildOf(parent));
        let entity = e.id();
        map.insert(path.clone(), entity);
        registry.project_prim(stage, &p, world, entity);
        spawned_count += 1;
    }

    // 4. Repatch existing prims (current_paths ∩ old_paths)
    let mut patched_count = 0usize;
    for path in &current_paths {
        if let Some(&entity) = old_entities.get(path) {
            if let Ok(p) = openusd::sdf::path(path) {
                registry.patch_prim(stage, &p, world, entity, &[]);
                patched_count += 1;
            }
        }
    }

    // 5. Maintain AnimatedPrims for the affected subtrees
    if let Some(mut animated_res) = world.get_resource_mut::<AnimatedPrims>() {
        // Remove existing animated paths under affected roots
        animated_res.0.retain(|anim_path| {
            !roots
                .iter()
                .any(|root| is_descendant_or_self(root, anim_path))
        });
        // Re-scan current paths in the subtrees
        for path in &current_paths {
            if let Ok(p) = openusd::sdf::path(path) {
                if prim_is_animated(stage, &p) {
                    animated_res.0.insert(path.clone());
                }
            }
        }
    }

    world.insert_resource(ReconcileStats {
        roots: roots.len(),
        visited_stage_prims: current_paths.len(),
        patched_entities: patched_count,
        spawned_entities: spawned_count,
        despawned_entities: despawned_count,
    });
}

/// Reconcile the projected entities against the stage's current prims (full stage):
/// despawn entities whose prim was removed, spawn entities for new prims,
/// patch transforms on the rest.
fn reconcile_full(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let stage = &live.stage;
    let registry = registry_of(world);
    let mut current: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Err(error) = stage.traverse(traverse_predicate(), |p: &openusd::sdf::Path| {
        current.insert(p.as_str().to_string());
    }) {
        bevy::log::error!(
            "[reconcile_full] stage traversal failed: {error:#}; aborting full reconcile without mutating entity mappings"
        );
        return;
    }

    // Despawn entities for prims no longer present (never the `/` stage root).
    let stale: Vec<(String, Entity)> = map
        .iter()
        .filter(|(p, _)| *p != "/" && !current.contains(*p))
        .map(|(p, e)| (p.to_string(), e))
        .collect();
    let despawned_count = stale.len();
    for (path, entity) in stale {
        if let Some(mut semantic_idx) = world.get_resource_mut::<SemanticEntityIndex>() {
            semantic_idx.remove_entity(entity);
        }
        world.despawn(entity);
        map.remove_path(&path);
    }

    // Spawn new prims (shallowest first, so a child finds its parent) and
    // reapply routes on existing ones. Both go through the registry: new prims
    // get a full `project_prim`; existing prims get a full re-patch (empty
    // `changed` = "reapply everything", the conservative choice on a resync).
    let root = map.entity("/").unwrap_or_else(|| {
        let r = world
            .spawn((
                UsdPrimRef {
                    path: "/".to_string(),
                },
                Transform::from_rotation(stage_up_axis(stage)),
                Visibility::default(),
            ))
            .id();
        map.insert("/", r);
        r
    });
    let mut ordered: Vec<&String> = current.iter().collect();
    ordered.sort_by_key(|p| p.matches('/').count());
    let mut animated: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut patched_count = 0usize;
    let mut spawned_count = 0usize;
    for path in ordered {
        let Ok(p) = openusd::sdf::path(path) else {
            continue;
        };
        if prim_is_animated(stage, &p) {
            animated.insert(path.clone());
        }
        if let Some(entity) = map.entity(path) {
            registry.patch_prim(stage, &p, world, entity, &[]);
            patched_count += 1;
        } else {
            let parent = map.entity(parent_path(path)).or(Some(root));
            let mut e = world.spawn(UsdPrimRef { path: path.clone() });
            if let Some(parent) = parent {
                e.insert(ChildOf(parent));
            }
            let entity = e.id();
            map.insert(path.clone(), entity);
            registry.project_prim(stage, &p, world, entity);
            spawned_count += 1;
        }
    }
    // Refresh the animated set for the reconciled prim set.
    world.insert_resource(AnimatedPrims(animated));
    world.insert_resource(ReconcileStats {
        roots: 1,
        visited_stage_prims: current.len(),
        patched_entities: patched_count,
        spawned_entities: spawned_count,
        despawned_entities: despawned_count,
    });
}

// ─── Bevy plugin + systems ──────────────────────────────────────────
//
// `LiveStage` is `!Send`, and `apply_changes`/`project_stage` need `&mut
// World` (to spawn/despawn) plus `&LiveStage` plus `&mut PrimEntities` at
// once — which would alias `World`. So the exclusive systems below
// temporarily *remove* the live stage + bimap from the world, run, and
// re-insert. An app does: `app.add_plugins(LiveStagePlugin)` then
// `world.insert_non_send(LiveStage::new(stage))` to start a session.

use bevy::app::{App, Plugin, Update};

/// Registers the `PrimEntities` bimap and the per-frame reprojection system.
/// Insert a `LiveStage` non-send resource to begin a live session.
pub struct LiveStagePlugin;

impl Plugin for LiveStagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PrimEntities>()
            .init_resource::<SemanticEntityIndex>()
            .init_resource::<PendingStageChanges>()
            .init_resource::<ReconcileStats>()
            .init_resource::<StageTime>()
            .init_resource::<AnimatedPrims>()
            .init_resource::<SampledTime>()
            .init_resource::<crate::route::DisplayPurposes>()
            .init_resource::<AppliedPurposes>()
            .add_systems(
                Update,
                (
                    project_on_load_system,
                    drain_stage_changes_system,
                    reproject_from_batch_system,
                    resample_animation_system,
                    apply_display_purposes_system,
                )
                    .chain(),
            );
        // Ensure the routing registry exists even if `UsdPlugin` wasn't added.
        if !app.world().contains_resource::<SchemaRegistry>() {
            app.insert_resource(SchemaRegistry::builtin());
        }
    }
}

/// One-shot projection the first frame a `LiveStage` is present.
fn project_on_load_system(world: &mut World) {
    if world.get_non_send::<LiveStage>().is_none() {
        return;
    }
    // Only project once per session: skip if the bimap is already populated.
    if let Some(map) = world.get_resource::<PrimEntities>() {
        if !map.is_empty() {
            return;
        }
    }
    let Some(live) = world.remove_non_send::<LiveStage>() else {
        return;
    };
    let mut map = world.remove_resource::<PrimEntities>().unwrap_or_default();
    project_stage(world, &live, &mut map);
    world.insert_resource(map);
    world.insert_non_send(live);
}

/// Resample animated prims when [`StageTime`] moves. Only revisits the prims
/// that actually carry time samples ([`AnimatedPrims`]), re-patching them at
/// the new time (the routes read `StageTime` when resolving values).
fn resample_animation_system(world: &mut World) {
    if world.get_non_send::<LiveStage>().is_none() {
        return;
    }
    let current = world.get_resource::<StageTime>().map(|t| t.current);
    let last = world.get_resource::<SampledTime>().and_then(|t| t.0);
    if current == last {
        return; // time hasn't moved
    }
    let animated = world
        .get_resource::<AnimatedPrims>()
        .cloned()
        .unwrap_or_default();
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
        if let Some(entity) = map.entity(path)
            && let Ok(p) = openusd::sdf::path(path)
        {
            // patch_prim resolves values at the world's StageTime.
            registry.patch_prim(&live.stage, &p, world, entity, &[]);
        }
    }
    world.insert_resource(map);
    world.insert_non_send(live);
    if let Some(mut sampled) = world.get_resource_mut::<SampledTime>() {
        sampled.0 = current;
    }
}

/// Re-filter every prim's visibility when [`DisplayPurposes`] changes (a
/// viewport toggling proxy↔render, or revealing guides). Purpose is inherited,
/// so a toggle can flip any prim — re-patch them all with a synthetic `purpose`
/// change; routes that don't own `purpose` ignore it.
fn apply_display_purposes_system(world: &mut World) {
    if world.get_non_send::<LiveStage>().is_none() {
        return;
    }
    let current = world
        .get_resource::<crate::route::DisplayPurposes>()
        .copied();
    let last = world.get_resource::<AppliedPurposes>().and_then(|a| a.0);
    if current == last {
        return; // toggle hasn't moved
    }
    let Some(live) = world.remove_non_send::<LiveStage>() else {
        return;
    };
    let map = world.remove_resource::<PrimEntities>().unwrap_or_default();
    let registry = registry_of(world);
    let entries: Vec<(String, Entity)> = map.iter().map(|(p, e)| (p.to_string(), e)).collect();
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

/// Drain the live stage's change queue once and publish the batch for this
/// frame's projection and future semantic consumers.
fn drain_stage_changes_system(world: &mut World) {
    let batch = world
        .get_non_send::<LiveStage>()
        .and_then(LiveStage::drain_change_batch);
    world.resource_mut::<PendingStageChanges>().batch = batch;
}

/// Reproject the batch published by [`drain_stage_changes_system`]. The batch
/// remains in [`PendingStageChanges`] so later consumers see the same data.
fn reproject_from_batch_system(world: &mut World) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippet::UsdSnippet;
    use openusd::usd::Stage;

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
    fn reconcile_synthetic_wide_scopes_to_resynced_subtree() {
        let mut usda = String::from("#usda 1.0\n\ndef Xform \"World\"\n{\n");
        for group in ["A", "B", "C"] {
            usda.push_str(&format!("    def Xform \"{group}\"\n    {{\n"));
            for i in 0..10 {
                usda.push_str(&format!(
                    "        def Xform \"{group}{i}\"\n        {{\n        }}\n"
                ));
            }
            usda.push_str("    }\n");
        }
        usda.push_str("}\n");

        let stage = crate::snippet::UsdSnippet::new(&usda)
            .open_stage()
            .expect("synthetic wide stage opens");
        let mut app = App::new();
        app.add_plugins(LiveStagePlugin);
        app.world_mut().insert_non_send(LiveStage::new(stage));

        // Initial frame performs initial project_stage
        app.update();
        assert_eq!(app.world().resource::<PrimEntities>().len(), 35);

        // Subtree resync targeting /World/B (1 root + 10 children = 11 prims)
        app.world()
            .get_non_send::<LiveStage>()
            .expect("live stage exists")
            .enqueue_resync("/World/B");
        app.update();

        let stats = *app.world().resource::<ReconcileStats>();
        assert_eq!(stats.roots, 1);
        assert_eq!(stats.visited_stage_prims, 11);
        assert_eq!(stats.patched_entities, 11);
        assert_eq!(stats.spawned_entities, 0);
        assert_eq!(stats.despawned_entities, 0);

        // All 35 entities remain mapped
        assert_eq!(app.world().resource::<PrimEntities>().len(), 35);
    }

    #[test]
    fn reconcile_deep_overlap_minimizes_roots_and_scopes_work() {
        let stage = crate::snippet::UsdSnippet::new(
            r#"#usda 1.0

def Xform "World"
{
    def Xform "A"
    {
        def Xform "Child"
        {
            def Xform "Leaf"
            {
            }
        }
    }
    def Xform "B"
    {
    }
    def Xform "C"
    {
    }
}
"#,
        )
        .open_stage()
        .expect("deep overlap stage opens");

        let mut app = App::new();
        app.add_plugins(LiveStagePlugin);
        app.world_mut().insert_non_send(LiveStage::new(stage));
        app.update();

        let live = app.world().get_non_send::<LiveStage>().unwrap();
        live.enqueue_resync("/World/A");
        live.enqueue_resync("/World/A/Child");
        live.enqueue_resync("/World/A/Child/Leaf");
        app.update();

        let stats = *app.world().resource::<ReconcileStats>();
        // Minimizes to 1 root (/World/A) and visits/patches only 3 prims (/World/A, /World/A/Child, /World/A/Child/Leaf)
        assert_eq!(stats.roots, 1);
        assert_eq!(stats.visited_stage_prims, 3);
        assert_eq!(stats.patched_entities, 3);
        assert_eq!(stats.spawned_entities, 0);
        assert_eq!(stats.despawned_entities, 0);
    }

    #[test]
    fn reconcile_real_materials_fixture_scopes_to_materials_subtree() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/stages/materials.usda");
        let stage =
            Stage::open(path.to_str().expect("valid path")).expect("materials fixture opens");

        let mut app = App::new();
        app.add_plugins(LiveStagePlugin);
        app.world_mut().insert_non_send(LiveStage::new(stage));
        app.update();

        let initial_count = app.world().resource::<PrimEntities>().len();
        assert_eq!(initial_count, 13); // 12 prims + root "/"

        app.world()
            .get_non_send::<LiveStage>()
            .unwrap()
            .enqueue_resync("/World/Materials");
        app.update();

        let stats = *app.world().resource::<ReconcileStats>();
        assert_eq!(stats.roots, 1);
        assert_eq!(stats.visited_stage_prims, 7);
        assert_eq!(stats.patched_entities, 7);
        assert_eq!(stats.spawned_entities, 0);
        assert_eq!(stats.despawned_entities, 0);
    }

    #[test]
    fn reconcile_subtree_spawns_and_despawns_while_preserving_sibling_entity_ids() {
        let stage = Stage::builder()
            .in_memory("subtree-spawn-despawn.usda")
            .expect("in-memory stage");

        stage.define_prim("/World").unwrap();
        stage.define_prim("/World/A").unwrap();
        stage.define_prim("/World/A/Child1").unwrap();
        stage.define_prim("/World/A/Child2").unwrap();
        stage.define_prim("/World/B").unwrap();

        let mut app = App::new();
        app.add_plugins(LiveStagePlugin);
        app.world_mut().insert_non_send(LiveStage::new(stage));
        app.update();

        let world_b_entity = app
            .world()
            .resource::<PrimEntities>()
            .entity("/World/B")
            .unwrap();
        let child1_entity = app
            .world()
            .resource::<PrimEntities>()
            .entity("/World/A/Child1")
            .unwrap();

        // Author changes in /World/A subtree: remove Child2, define Child3
        let live = app.world().get_non_send::<LiveStage>().unwrap();
        live.stage.remove_prim("/World/A/Child2").unwrap();
        live.stage.define_prim("/World/A/Child3").unwrap();
        let _ = live.drain_change_batch();
        live.enqueue_resync("/World/A");

        app.update();

        let stats = *app.world().resource::<ReconcileStats>();
        assert_eq!(stats.roots, 1);
        assert_eq!(stats.visited_stage_prims, 3); // /World/A, /World/A/Child1, /World/A/Child3
        assert_eq!(stats.patched_entities, 2); // /World/A, /World/A/Child1
        assert_eq!(stats.spawned_entities, 1); // /World/A/Child3
        assert_eq!(stats.despawned_entities, 1); // /World/A/Child2

        // Verify entity preservation and removal
        let prim_entities = app.world().resource::<PrimEntities>();
        assert_eq!(prim_entities.entity("/World/B"), Some(world_b_entity));
        assert_eq!(prim_entities.entity("/World/A/Child1"), Some(child1_entity));
        assert!(prim_entities.entity("/World/A/Child2").is_none());
        assert!(prim_entities.entity("/World/A/Child3").is_some());
    }

    #[test]
    fn test_reconcile_subtrees_missing_external_parent_triggers_full_fallback() {
        let stage = Stage::builder()
            .in_memory("missing-external-parent.usda")
            .expect("in-memory stage");

        stage.define_prim("/World").unwrap();
        stage.define_prim("/World/A").unwrap();

        let mut app = App::new();
        app.add_plugins(LiveStagePlugin);
        app.world_mut().insert_non_send(LiveStage::new(stage));
        app.update();

        // Simulate corrupted map where external parent /World/A was removed from PrimEntities
        app.world_mut()
            .resource_mut::<PrimEntities>()
            .remove_path("/World/A");

        // Define a new child /World/A/B on stage
        let live = app.world().get_non_send::<LiveStage>().unwrap();
        live.stage.define_prim("/World/A/B").unwrap();
        let _ = live.drain_change_batch();
        // Enqueue resync scoped to /World/A/B
        live.enqueue_resync("/World/A/B");

        app.update();

        // Preflight detected missing external parent /World/A and aborted subtree reconcile,
        // falling back to reconcile_full which restored the complete hierarchy.
        let prim_entities = app.world().resource::<PrimEntities>();
        let a_entity = prim_entities.entity("/World/A").expect("/World/A restored");
        let b_entity = prim_entities
            .entity("/World/A/B")
            .expect("/World/A/B spawned");

        // Verify /World/A/B is child of /World/A, NOT child of stage root "/"
        let b_child_of = app.world().get::<ChildOf>(b_entity).expect("has ChildOf");
        assert_eq!(b_child_of.parent(), a_entity);
    }

    #[test]
    fn test_reconcile_subtrees_missing_stage_root_triggers_full_fallback() {
        let stage = Stage::builder()
            .in_memory("missing-stage-root.usda")
            .expect("in-memory stage");

        stage.define_prim("/World").unwrap();

        let mut app = App::new();
        app.add_plugins(LiveStagePlugin);
        app.world_mut().insert_non_send(LiveStage::new(stage));
        app.update();

        // Simulate missing stage root "/"
        app.world_mut()
            .resource_mut::<PrimEntities>()
            .remove_path("/");

        let live = app.world().get_non_send::<LiveStage>().unwrap();
        live.stage.define_prim("/World/NewPrim").unwrap();
        let _ = live.drain_change_batch();
        live.enqueue_resync("/World/NewPrim");

        app.update();

        // Subtree reconcile falls back to full reconcile and restores "/"
        let prim_entities = app.world().resource::<PrimEntities>();
        assert!(prim_entities.entity("/").is_some());
        assert!(prim_entities.entity("/World/NewPrim").is_some());
    }

    #[test]
    fn test_reconcile_full_resync_root_path_reconciles_entire_stage() {
        let stage = Stage::builder()
            .in_memory("full-reconcile-root.usda")
            .expect("in-memory stage");

        stage.define_prim("/World").unwrap();
        stage.define_prim("/World/A").unwrap();
        stage.define_prim("/World/B").unwrap();

        let mut app = App::new();
        app.add_plugins(LiveStagePlugin);
        app.world_mut().insert_non_send(LiveStage::new(stage));
        app.update();

        assert_eq!(app.world().resource::<PrimEntities>().len(), 4); // /, /World, /World/A, /World/B

        // Enqueue resync on "/" which directly invokes reconcile_full
        let live = app.world().get_non_send::<LiveStage>().unwrap();
        live.enqueue_resync("/");
        app.update();

        let stats = *app.world().resource::<ReconcileStats>();
        assert_eq!(stats.roots, 1);
        assert_eq!(stats.visited_stage_prims, 3); // /World, /World/A, /World/B
        assert_eq!(stats.patched_entities, 3);
        assert_eq!(stats.spawned_entities, 0);
        assert_eq!(stats.despawned_entities, 0);
    }

    #[test]
    fn test_reconcile_subtrees_maintains_animated_prims_scoped_to_subtree() {
        let usda = r#"#usda 1.0
(
    startTimeCode = 0
    endTimeCode = 10
)

def Xform "World"
{
    def Xform "AnimOutside"
    {
        double3 xformOp:translate.timeSamples = {
            0: (0, 0, 0),
            10: (10, 0, 0),
        }
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }
    def Xform "A"
    {
        def Xform "AnimInsideOld"
        {
            double3 xformOp:translate.timeSamples = {
                0: (0, 0, 0),
                10: (0, 5, 0),
            }
            uniform token[] xformOpOrder = ["xformOp:translate"]
        }
    }
}
"#;

        let stage = crate::snippet::UsdSnippet::new(usda)
            .open_stage()
            .expect("animated stage opens");
        let mut app = App::new();
        app.add_plugins(LiveStagePlugin);
        app.world_mut().insert_non_send(LiveStage::new(stage));
        app.update();

        let anim = app.world().resource::<AnimatedPrims>();
        assert!(anim.0.contains("/World/AnimOutside"));
        assert!(anim.0.contains("/World/A/AnimInsideOld"));

        // Remove /World/A/AnimInsideOld and define /World/A/StaticNew
        let live = app.world().get_non_send::<LiveStage>().unwrap();
        live.stage.remove_prim("/World/A/AnimInsideOld").unwrap();
        live.stage.define_prim("/World/A/StaticNew").unwrap();

        let _ = live.drain_change_batch();
        live.enqueue_resync("/World/A");
        app.update();

        let anim_after = app.world().resource::<AnimatedPrims>();
        // Unaffected outside animated path is preserved
        assert!(anim_after.0.contains("/World/AnimOutside"));
        // Old subtree animated path was cleaned
        assert!(!anim_after.0.contains("/World/A/AnimInsideOld"));
        // Static new path is not animated
        assert!(!anim_after.0.contains("/World/A/StaticNew"));
    }

    #[test]
    fn test_reconcile_subtrees_maintains_semantic_entity_index() {
        use usd_model::EntityKey;

        let stage = Stage::builder()
            .in_memory("semantic-index-subtree.usda")
            .expect("in-memory stage");

        stage.define_prim("/World").unwrap();
        stage.define_prim("/World/A").unwrap();
        stage.define_prim("/World/A/Child").unwrap();
        stage.define_prim("/World/B").unwrap();

        let mut app = App::new();
        app.add_plugins(LiveStagePlugin);
        app.world_mut().insert_non_send(LiveStage::new(stage));
        app.update();

        let world_b_entity = app
            .world()
            .resource::<PrimEntities>()
            .entity("/World/B")
            .unwrap();
        let child_entity = app
            .world()
            .resource::<PrimEntities>()
            .entity("/World/A/Child")
            .unwrap();

        let key_b = EntityKey::new("entity_b");
        let key_child = EntityKey::new("entity_child");

        // Register semantic keys
        {
            let mut semantic_index = app.world_mut().resource_mut::<SemanticEntityIndex>();
            semantic_index.insert(key_b.clone(), world_b_entity);
            semantic_index.insert(key_child.clone(), child_entity);
        }

        // Remove /World/A/Child and enqueue resync on /World/A
        let live = app.world().get_non_send::<LiveStage>().unwrap();
        live.stage.remove_prim("/World/A/Child").unwrap();
        let _ = live.drain_change_batch();
        live.enqueue_resync("/World/A");

        app.update();

        let semantic_index = app.world().resource::<SemanticEntityIndex>();
        // Sibling outside subtree remains mapped
        assert_eq!(semantic_index.entity(&key_b), Some(world_b_entity));
        assert_eq!(semantic_index.key(world_b_entity), Some(&key_b));

        // Despawned entity mapping is completely cleaned
        assert!(semantic_index.entity(&key_child).is_none());
        assert!(semantic_index.key(child_entity).is_none());
    }

    #[test]
    fn test_normalize_prim_path() {
        assert_eq!(normalize_prim_path(""), "/");
        assert_eq!(normalize_prim_path("   "), "/");
        assert_eq!(normalize_prim_path("/"), "/");
        assert_eq!(normalize_prim_path("/World"), "/World");
        assert_eq!(normalize_prim_path("/World/"), "/World");
        assert_eq!(normalize_prim_path("World/A/B"), "/World/A/B");
        assert_eq!(normalize_prim_path("/World/A.property"), "/World/A");
        assert_eq!(
            normalize_prim_path("/World/Robot.userProperties:name"),
            "/World/Robot"
        );
        assert_eq!(
            normalize_prim_path("/World/A/B.xformOp:transform"),
            "/World/A/B"
        );
    }

    #[test]
    fn test_is_descendant_or_self() {
        // Root / covers all paths
        assert!(is_descendant_or_self("/", "/"));
        assert!(is_descendant_or_self("/", "/World"));
        assert!(is_descendant_or_self("/", "/World/A/B"));

        // Exact match
        assert!(is_descendant_or_self("/World/A", "/World/A"));

        // True descendants
        assert!(is_descendant_or_self("/World/A", "/World/A/B"));
        assert!(is_descendant_or_self("/World/A", "/World/A/B/Leaf"));
        assert!(is_descendant_or_self("/World/A", "/World/A.property"));

        // Boundary awareness (avoiding prefix collisions)
        assert!(!is_descendant_or_self("/World/A", "/World/AB"));
        assert!(!is_descendant_or_self("/World/A", "/World/A_Other"));
        assert!(!is_descendant_or_self("/World/A", "/World/B"));
        assert!(!is_descendant_or_self("/World/A", "/World"));
    }

    #[test]
    fn test_minimize_resync_roots() {
        // Empty
        assert_eq!(
            minimize_resync_roots(Vec::<&str>::new()),
            Vec::<String>::new()
        );

        // Deduplication
        assert_eq!(
            minimize_resync_roots(["/World/A", "/World/A"]),
            vec!["/World/A".to_string()]
        );

        // Deep overlap minimization
        let input = [
            "/World/A/B",
            "/World/C",
            "/World/A",
            "/World/A/B/Leaf",
            "/World/C/Sub",
        ];
        let result = minimize_resync_roots(input);
        assert_eq!(result, vec!["/World/A".to_string(), "/World/C".to_string()]);

        // Prefix boundary respected
        let input = ["/World/A", "/World/AB", "/World/A/Child"];
        let result = minimize_resync_roots(input);
        assert_eq!(
            result,
            vec!["/World/A".to_string(), "/World/AB".to_string()]
        );

        // Full stage root covers all
        let input = ["/World/A", "/World/B", "/", "/World/C/D"];
        let result = minimize_resync_roots(input);
        assert_eq!(result, vec!["/".to_string()]);

        // Property paths stripped to owning prims
        let input = ["/World/A.xformOp:transform", "/World/A/Child.property"];
        let result = minimize_resync_roots(input);
        assert_eq!(result, vec!["/World/A".to_string()]);
    }

    #[test]
    fn test_stage_change_batch_resync_roots_and_unshaded_changed_info() {
        let batch = StageChangeBatch {
            revision: LiveRevision(1),
            changes: vec![
                StageChange {
                    resynced: vec![
                        "/World/A/Child".to_string(),
                        "/World/A".to_string(),
                        "/World/C".to_string(),
                    ],
                    changed_info: vec![
                        "/World/A/Child.userProperties:speed".to_string(),
                        "/World/B.userProperties:name".to_string(),
                        "/World/C/Leaf.xformOp:transform".to_string(),
                        "/World/D.visibility".to_string(),
                    ],
                },
                StageChange {
                    resynced: vec!["/World/C/Sub".to_string()],
                    changed_info: vec!["/World/D.visibility".to_string()], // duplicate
                },
            ],
        };

        assert!(batch.has_resync());
        assert_eq!(
            batch.resync_roots(),
            vec!["/World/A".to_string(), "/World/C".to_string()]
        );
        assert!(batch.is_path_under_resync("/World/A"));
        assert!(batch.is_path_under_resync("/World/A/Child/Leaf"));
        assert!(batch.is_path_under_resync("/World/C"));
        assert!(!batch.is_path_under_resync("/World/B"));
        assert!(!batch.is_path_under_resync("/World/D"));

        // /World/A/... and /World/C/... are shaded by resync roots /World/A and /World/C
        let unshaded = batch.unshaded_changed_info();
        assert_eq!(
            unshaded,
            vec![
                "/World/B.userProperties:name".to_string(),
                "/World/D.visibility".to_string(),
            ]
        );
    }

    #[test]
    fn test_collect_stage_subtree_paths_synthetic_wide() {
        let mut usda = String::from("#usda 1.0\n\ndef Xform \"World\"\n{\n");
        for group in ["A", "B", "C"] {
            usda.push_str(&format!("    def Xform \"{group}\"\n    {{\n"));
            for i in 0..10 {
                usda.push_str(&format!(
                    "        def Xform \"{group}{i}\"\n        {{\n        }}\n"
                ));
            }
            usda.push_str("    }\n");
        }
        usda.push_str("}\n");

        let stage = UsdSnippet::new(&usda)
            .open_stage()
            .expect("synthetic wide stage opens");

        // Subtree /World/B has 1 root + 10 children = 11 prims
        let b_paths = collect_stage_subtree_paths(&stage, "/World/B").expect("collect /World/B");
        assert_eq!(b_paths.len(), 11);
        assert_eq!(b_paths[0], "/World/B");
        for i in 0..10 {
            assert!(b_paths.contains(&format!("/World/B/B{i}")));
        }

        // Leaf prim /World/A/A0 has 1 prim
        let leaf_paths =
            collect_stage_subtree_paths(&stage, "/World/A/A0").expect("collect /World/A/A0");
        assert_eq!(leaf_paths, vec!["/World/A/A0".to_string()]);

        // Full stage root "/" collects all 34 prims
        let all_paths = collect_stage_subtree_paths(&stage, "/").expect("collect /");
        assert_eq!(all_paths.len(), 34);

        // Non-existent subtree returns empty
        let missing =
            collect_stage_subtree_paths(&stage, "/World/NonExistent").expect("collect missing");
        assert!(missing.is_empty());
    }

    #[test]
    fn test_collect_stage_subtree_paths_deep_overlap() {
        let stage = UsdSnippet::new(
            r#"#usda 1.0

def Xform "World"
{
    def Xform "A"
    {
        def Xform "Child"
        {
            def Xform "Leaf"
            {
            }
        }
    }
    def Xform "B"
    {
    }
}
"#,
        )
        .open_stage()
        .expect("deep overlap stage opens");

        let a_paths = collect_stage_subtree_paths(&stage, "/World/A").expect("collect /World/A");
        assert_eq!(
            a_paths,
            vec![
                "/World/A".to_string(),
                "/World/A/Child".to_string(),
                "/World/A/Child/Leaf".to_string(),
            ]
        );

        let child_paths =
            collect_stage_subtree_paths(&stage, "/World/A/Child").expect("collect /World/A/Child");
        assert_eq!(
            child_paths,
            vec![
                "/World/A/Child".to_string(),
                "/World/A/Child/Leaf".to_string(),
            ]
        );
    }

    #[test]
    fn test_collect_stage_subtree_paths_respects_projection_predicate() {
        let stage = UsdSnippet::new(
            r#"#usda 1.0

def Xform "World"
{
    def Xform "Visible"
    {
    }
    class "_AbstractBase"
    {
        def Xform "UnderAbstract"
        {
        }
    }
}
"#,
        )
        .open_stage()
        .expect("stage opens");

        let paths = collect_stage_subtree_paths(&stage, "/").expect("collect /");
        assert!(paths.contains(&"/World".to_string()));
        assert!(paths.contains(&"/World/Visible".to_string()));
        // Abstract classes should be excluded by the projection predicate
        assert!(!paths.iter().any(|p| p.contains("_AbstractBase")));
    }
}

// ─── Authoring back (entity edit → stage) ───────────────────────────
//
// The write direction: an entity's `Transform` (e.g. after a gizmo drag)
// authored back onto the prim as a single `xformOp:transform` matrix under
// the stage's current edit target. The commit fires the sink, so the edit
// re-projects like any other change (idempotent — the entity already holds
// the value). Authoring one matrix op (instead of decomposed T/R/S) keeps a
// clean round-trip with `read_transform`.

/// Author `transform` onto `prim_path` as `xformOp:transform`. Errors if the
/// path is malformed or the layer rejects the edit.
pub fn author_transform(
    stage: &Stage,
    prim_path: &str,
    transform: &Transform,
) -> anyhow::Result<()> {
    use openusd::sdf::Value;
    let prim = openusd::sdf::path(prim_path)?;
    let cols = Mat4::from_scale_rotation_translation(
        transform.scale,
        transform.rotation,
        transform.translation,
    )
    .to_cols_array();
    let m: [f64; 16] = std::array::from_fn(|i| cols[i] as f64);

    let xop = prim.append_property("xformOp:transform")?;
    stage
        .create_attribute(xop, "matrix4d")?
        .set(Value::Matrix4d(openusd::gf::Matrix4d(m)))?;
    let order = prim.append_property("xformOpOrder")?;
    stage
        .create_attribute(order, "token[]")?
        .set(Value::TokenVec(vec!["xformOp:transform".into()]))?;
    Ok(())
}

/// Current authored transform of a prim, if any.
pub fn current_transform(stage: &Stage, prim_path: &str) -> Option<Transform> {
    openusd::sdf::path(prim_path)
        .ok()
        .and_then(|p| read_transform(stage, &p).ok().flatten())
        .map(to_bevy_transform)
}

fn clear_transform(stage: &Stage, prim_path: &str) -> anyhow::Result<()> {
    let prim = openusd::sdf::path(prim_path)?;
    let _ = stage.remove_property(prim.append_property("xformOp:transform")?);
    let _ = stage.remove_property(prim.append_property("xformOpOrder")?);
    Ok(())
}

// ─── Undo / redo for transform edits (RETHINK P6, gizmo slice) ───────
//
// Typed-action history: each edit captures the prim's transform before +
// after, so undo re-authors the prior state (or clears it if there was
// none) and redo re-applies. General attribute / namespace undo via
// openusd `Diff` inverses is the next layer.

struct TransformEdit {
    prim: String,
    before: Option<Transform>,
    after: Transform,
}

/// Undo/redo stack for transform edits.
#[derive(Default)]
pub struct TransformHistory {
    undo: Vec<TransformEdit>,
    redo: Vec<TransformEdit>,
}

impl TransformHistory {
    /// Author `after` onto `prim`, recording the prior transform for undo.
    pub fn author(&mut self, stage: &Stage, prim: &str, after: Transform) -> anyhow::Result<()> {
        let before = current_transform(stage, prim);
        author_transform(stage, prim, &after)?;
        self.undo.push(TransformEdit {
            prim: prim.to_string(),
            before,
            after,
        });
        self.redo.clear();
        Ok(())
    }

    /// Undo the most recent edit. Returns `false` if nothing to undo.
    pub fn undo(&mut self, stage: &Stage) -> anyhow::Result<bool> {
        let Some(edit) = self.undo.pop() else {
            return Ok(false);
        };
        match &edit.before {
            Some(t) => author_transform(stage, &edit.prim, t)?,
            None => clear_transform(stage, &edit.prim)?,
        }
        self.redo.push(edit);
        Ok(true)
    }

    /// Redo the most recently undone edit. Returns `false` if nothing to redo.
    pub fn redo(&mut self, stage: &Stage) -> anyhow::Result<bool> {
        let Some(edit) = self.redo.pop() else {
            return Ok(false);
        };
        author_transform(stage, &edit.prim, &edit.after)?;
        self.undo.push(edit);
        Ok(true)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
