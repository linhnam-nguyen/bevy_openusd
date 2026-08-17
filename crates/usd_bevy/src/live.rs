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

impl StageChangeBatch {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
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
    fn enqueue_resync(&self, prim: &str) {
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
        let with_slash = format!("{prefix}/");
        self.by_path
            .iter()
            .filter(|(p, _)| p.as_str() == prefix || p.starts_with(&with_slash))
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
fn prim_of(path: &str) -> &str {
    path.split('.').next().unwrap_or(path)
}

/// The property part of a (possibly property) path: `/Foo.xformOp:x` →
/// `Some("xformOp:x")`; a bare prim path → `None`.
fn property_of(path: &str) -> Option<&str> {
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
    let registry = registry_of(world);
    // The stage-root entity (the pseudo-root `/`) carries the up-axis rotation;
    // every top-level prim hangs off it, so Bevy's transform propagation
    // composes prim-local transforms into correct world transforms and the
    // whole scene stands upright on the grid. It is not a real prim, so no
    // routes run on it (they would clobber the up-axis rotation).
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

    let mut prim_count = 0usize;
    let mut animated: std::collections::HashSet<String> = std::collections::HashSet::new();
    let _ = stage.traverse(traverse_predicate(), |path: &openusd::sdf::Path| {
        // Traversal is pre-order, so the parent prim's entity already exists.
        let parent = map.entity(parent_path(path.as_str())).unwrap_or(root);
        let entity = world
            .spawn((
                UsdPrimRef {
                    path: path.as_str().to_string(),
                },
                ChildOf(parent),
            ))
            .id();
        map.insert(path.as_str().to_string(), entity);
        prim_count += 1;
        if prim_is_animated(stage, path) {
            animated.insert(path.as_str().to_string());
        }
        // Every prim→component mapping goes through the registry.
        registry.project_prim(stage, path, world, entity);
    });
    bevy::log::info!(
        target: "usd_bevy::live",
        "projected {prim_count} prims ({} animated)",
        animated.len()
    );
    world.insert_resource(AnimatedPrims(animated));
    // Projecting authored the initial read; clear so the first sync starts clean.
    let _ = live.drain_change_batch();
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
    if batch.changes.iter().any(|c| !c.resynced.is_empty()) {
        reconcile(world, live, map);
        return;
    }
    // `changed_info` only: group the changed *property* paths by owning prim so
    // each route sees exactly which properties changed and can patch sparsely.
    let registry = registry_of(world);
    // Echo guard: prims we just authored ourselves are swallowed this round.
    let suppressed = live.take_suppressed();
    let mut by_prim: HashMap<String, Vec<String>> = HashMap::new();
    for change in &batch.changes {
        for path in change.paths() {
            let prim = prim_of(path).to_string();
            let entry = by_prim.entry(prim).or_default();
            if let Some(prop) = property_of(path) {
                entry.push(prop.to_string());
            }
        }
    }
    for (prim, props) in by_prim {
        if suppressed.contains(&prim) {
            continue;
        }
        let Some(entity) = map.entity(&prim) else {
            continue;
        };
        let Ok(p) = openusd::sdf::path(&prim) else {
            continue;
        };
        let prop_refs: Vec<&str> = props.iter().map(String::as_str).collect();
        registry.patch_prim(&live.stage, &p, world, entity, &prop_refs);
    }
}

/// Reconcile the projected entities against the stage's current prims:
/// despawn entities whose prim was removed, spawn entities for new prims,
/// patch transforms on the rest.
fn reconcile(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let stage = &live.stage;
    let registry = registry_of(world);
    let mut current: std::collections::HashSet<String> = std::collections::HashSet::new();
    let _ = stage.traverse(traverse_predicate(), |p: &openusd::sdf::Path| {
        current.insert(p.as_str().to_string());
    });

    // Despawn entities for prims no longer present (never the `/` stage root).
    let stale: Vec<(String, Entity)> = map
        .iter()
        .filter(|(p, _)| *p != "/" && !current.contains(*p))
        .map(|(p, e)| (p.to_string(), e))
        .collect();
    for (path, entity) in stale {
        world.despawn(entity);
        map.remove_path(&path);
    }

    // Spawn new prims (shallowest first, so a child finds its parent) and
    // reapply routes on existing ones. Both go through the registry: new prims
    // get a full `project_prim`; existing prims get a full re-patch (empty
    // `changed` = "reapply everything", the conservative choice on a resync).
    let root = map.entity("/");
    let mut ordered: Vec<&String> = current.iter().collect();
    ordered.sort_by_key(|p| p.matches('/').count());
    let mut animated: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in ordered {
        let Ok(p) = openusd::sdf::path(path) else {
            continue;
        };
        if prim_is_animated(stage, &p) {
            animated.insert(path.clone());
        }
        if let Some(entity) = map.entity(path) {
            registry.patch_prim(stage, &p, world, entity, &[]);
        } else {
            let parent = map.entity(parent_path(path)).or(root);
            let mut e = world.spawn(UsdPrimRef { path: path.clone() });
            if let Some(parent) = parent {
                e.insert(ChildOf(parent));
            }
            let entity = e.id();
            map.insert(path.clone(), entity);
            registry.project_prim(stage, &p, world, entity);
        }
    }
    // Refresh the animated set for the reconciled prim set.
    world.insert_resource(AnimatedPrims(animated));
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
    if !world.resource::<PrimEntities>().is_empty() {
        return;
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
