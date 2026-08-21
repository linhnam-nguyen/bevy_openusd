use bevy::mesh::Mesh3d;
use bevy::prelude::*;
use std::collections::HashSet;
use std::time::{Duration, Instant};

use super::animation::{AnimatedPrims, prim_is_animated};
use super::index::PrimEntities;
use super::progressive_cleanup::clear_projection;
use super::progressive_resident::resident_projection;
use super::projection::{ProjectionStats, project_plan_entry, registry_of};
use super::projection_plan::ProjectionPlan;
use super::stage::LiveStage;

/// A bounded amount of projection work for one Bevy update.
#[derive(Resource, Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectionBudget {
    /// Maximum number of plan entries to project in one update.
    pub max_work_items: Option<usize>,
    /// Optional wall-clock limit checked between plan entries.
    pub max_duration: Option<Duration>,
}

impl ProjectionBudget {
    /// Disable both limits. This is the compatibility/default mode.
    pub const fn unlimited() -> Self {
        Self {
            max_work_items: None,
            max_duration: None,
        }
    }

    /// Limit one update to at most `items` entries.
    pub const fn work_items(items: usize) -> Self {
        Self {
            max_work_items: Some(items),
            max_duration: None,
        }
    }

    /// Limit one update by elapsed wall-clock time.
    pub fn time(duration: Duration) -> Self {
        Self {
            max_work_items: None,
            max_duration: Some(duration),
        }
    }
}

impl Default for ProjectionBudget {
    fn default() -> Self {
        Self::unlimited()
    }
}

/// Additive readiness state for progressive projection. This does not replace
/// any application-level stage-load state; it only describes this projection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProjectionReadiness {
    #[default]
    Idle,
    Planning,
    Projecting,
    Ready,
    Cancelled,
    Failed,
}

/// Session-owned progress and generation state for the initial projection.
#[derive(Resource, Clone, Debug, Default)]
pub struct ProgressiveProjectionState {
    generation: u64,
    session_id: Option<u64>,
    readiness: ProjectionReadiness,
    completed: usize,
    total: usize,
    plan: Option<ProjectionPlan>,
    next_index: usize,
    entities: Vec<Option<Entity>>,
    animated: HashSet<String>,
    started_at: Option<Instant>,
    first_projected_prim_ms: Option<f64>,
    first_mesh_ms: Option<f64>,
    restart_requested: bool,
    last_error: Option<String>,
    cancelled_generations: u64,
    plan_builds: u64,
    resident_short_circuits: u64,
    resident_validation_requested: bool,
}

impl ProgressiveProjectionState {
    /// Current generation; a new stage session always advances it.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Current live-stage session owner.
    pub fn session_id(&self) -> Option<u64> {
        self.session_id
    }

    /// Current additive projection readiness.
    pub fn readiness(&self) -> ProjectionReadiness {
        self.readiness
    }

    /// Number of completed plan entries.
    pub fn completed(&self) -> usize {
        self.completed
    }

    /// Total plan entries, including the synthetic root.
    pub fn total(&self) -> usize {
        self.total
    }

    /// Completion ratio in `[0, 1]`.
    pub fn progress(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.completed as f32 / self.total as f32
        }
    }

    /// The current deterministic plan, retained for diagnostics after ready.
    pub fn plan(&self) -> Option<&ProjectionPlan> {
        self.plan.as_ref()
    }

    /// Time from planning start to the first projected non-root prim.
    pub fn first_projected_prim_ms(&self) -> Option<f64> {
        self.first_projected_prim_ms
    }

    /// Time from planning start to the first entity carrying `Mesh3d`.
    pub fn first_mesh_ms(&self) -> Option<f64> {
        self.first_mesh_ms
    }

    /// Number of cancelled generations in this resource lifetime.
    pub fn cancelled_generations(&self) -> u64 {
        self.cancelled_generations
    }

    /// Number of plans built, including reload/restart plans.
    pub fn plan_builds(&self) -> u64 {
        self.plan_builds
    }

    /// Number of resident-projection short circuits.
    pub fn resident_short_circuits(&self) -> u64 {
        self.resident_short_circuits
    }

    /// Request one validation of resident mesh/material handles on the next
    /// ready-state update. The normal idle path remains constant-time.
    pub fn invalidate_resident_cache(&mut self) {
        self.resident_validation_requested = true;
    }

    /// Last planning/projection error, if any.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Cancel the active generation. The next update starts a fresh plan and
    /// clears partial entities before projecting the current stage.
    pub fn cancel(&mut self) {
        if self.readiness == ProjectionReadiness::Projecting {
            self.readiness = ProjectionReadiness::Cancelled;
            self.restart_requested = true;
            self.cancelled_generations += 1;
        }
    }
}

/// The exclusive system that owns initial projection planning and draining.
pub(super) fn project_on_load_system(world: &mut World) {
    let Some((session_id, has_changes)) = world
        .get_non_send::<LiveStage>()
        .map(|live| (live.session_id(), live.has_changes()))
    else {
        return;
    };
    let (state_session, readiness, restart_requested, total, validate_resident) = {
        let state = world.resource::<ProgressiveProjectionState>();
        (
            state.session_id,
            state.readiness,
            state.restart_requested,
            state.total,
            state.resident_validation_requested,
        )
    };
    let map_len = world.resource::<PrimEntities>().len();
    if readiness == ProjectionReadiness::Projecting && has_changes {
        world
            .resource_mut::<ProgressiveProjectionState>()
            .restart_requested = true;
        return;
    }
    let resident = state_session == Some(session_id)
        && readiness == ProjectionReadiness::Ready
        && total == map_len
        && (!validate_resident
            || resident_projection(
                world,
                world.resource::<PrimEntities>(),
                world.resource::<ProgressiveProjectionState>(),
            ));
    if resident {
        let mut state = world.resource_mut::<ProgressiveProjectionState>();
        state.resident_short_circuits += 1;
        state.resident_validation_requested = false;
        return;
    }
    if state_session == Some(session_id)
        && readiness == ProjectionReadiness::Projecting
        && !has_changes
    {
        let Some(live) = world.remove_non_send::<LiveStage>() else {
            return;
        };
        let mut map = world.remove_resource::<PrimEntities>().unwrap_or_default();
        drain_generation(world, &live, &mut map);
        world.insert_resource(map);
        world.insert_non_send(live);
        return;
    }
    let needs_start = state_session != Some(session_id)
        || restart_requested
        || (readiness == ProjectionReadiness::Idle && map_len == 0)
        || (readiness == ProjectionReadiness::Cancelled && !has_changes)
        || (readiness == ProjectionReadiness::Ready && !resident);
    if !needs_start || has_changes {
        return;
    }

    let Some(live) = world.remove_non_send::<LiveStage>() else {
        return;
    };
    let mut map = world.remove_resource::<PrimEntities>().unwrap_or_default();
    start_generation(world, &live, &mut map, session_id);
    drain_generation(world, &live, &mut map);
    world.insert_resource(map);
    world.insert_non_send(live);
}

fn start_generation(world: &mut World, live: &LiveStage, map: &mut PrimEntities, session_id: u64) {
    let should_clear = world
        .resource::<ProgressiveProjectionState>()
        .session_id
        .is_some_and(|current| current != session_id)
        || !map.is_empty();
    if should_clear {
        clear_projection(world, map);
    }

    let plan_result = ProjectionPlan::from_stage(&live.stage);
    world.insert_resource(AnimatedPrims::default());
    let mut state = world.resource_mut::<ProgressiveProjectionState>();
    state.readiness = ProjectionReadiness::Planning;
    state.plan_builds += 1;
    state.last_error = None;
    let Ok(plan) = plan_result else {
        state.readiness = ProjectionReadiness::Failed;
        state.last_error = Some("failed to build deterministic projection plan".to_string());
        return;
    };
    state.generation = state
        .generation
        .checked_add(1)
        .expect("projection generation exhausted");
    state.session_id = Some(session_id);
    state.readiness = ProjectionReadiness::Projecting;
    state.completed = 0;
    state.total = plan.len();
    state.next_index = 0;
    state.entities = vec![None; plan.len()];
    state.animated.clear();
    state.started_at = Some(Instant::now());
    state.first_projected_prim_ms = None;
    state.first_mesh_ms = None;
    state.restart_requested = false;
    state.resident_validation_requested = false;
    state.plan = Some(plan);
    world.insert_resource(ProjectionStats::default());
}

fn drain_generation(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let budget = *world.resource::<ProjectionBudget>();
    let started = Instant::now();
    let mut processed = 0usize;
    let registry = registry_of(world);
    loop {
        if budget
            .max_work_items
            .is_some_and(|limit| processed >= limit)
            || budget
                .max_duration
                .is_some_and(|limit| started.elapsed() >= limit)
        {
            break;
        }
        let (index, entry, parent) = {
            let state = world.resource::<ProgressiveProjectionState>();
            let Some(entry) = state
                .plan
                .as_ref()
                .and_then(|plan| plan.entry(state.next_index))
                .cloned()
            else {
                break;
            };
            let parent = entry
                .parent_index()
                .and_then(|parent| state.entities.get(parent).copied().flatten());
            (state.next_index, entry, parent)
        };
        let entity = project_plan_entry(world, &live.stage, &registry, map, &entry, parent);
        let is_mesh = world.get::<Mesh3d>(entity).is_some();
        if entry.path() != "/"
            && openusd::sdf::path(entry.path())
                .ok()
                .is_some_and(|path| prim_is_animated(&live.stage, &path))
        {
            world
                .resource_mut::<ProgressiveProjectionState>()
                .animated
                .insert(entry.path().to_string());
        }
        let elapsed_ms = world
            .resource::<ProgressiveProjectionState>()
            .started_at
            .map(|start| start.elapsed().as_secs_f64() * 1000.0);
        let mut state = world.resource_mut::<ProgressiveProjectionState>();
        state.entities[index] = Some(entity);
        state.next_index += 1;
        state.completed += 1;
        if entry.path() != "/" && state.first_projected_prim_ms.is_none() {
            state.first_projected_prim_ms = elapsed_ms;
        }
        if is_mesh && state.first_mesh_ms.is_none() {
            state.first_mesh_ms = elapsed_ms;
        }
        processed += 1;
    }
    finish_if_ready(world, live);
}

fn finish_if_ready(world: &mut World, live: &LiveStage) {
    let (ready, animated, duration_ms, prims) = {
        let mut state = world.resource_mut::<ProgressiveProjectionState>();
        let complete = state.completed == state.total;
        if !complete {
            return;
        }
        state.readiness = ProjectionReadiness::Ready;
        let duration_ms = state
            .started_at
            .map(|start| start.elapsed().as_secs_f64() * 1000.0);
        (
            true,
            std::mem::take(&mut state.animated),
            duration_ms,
            state.total.saturating_sub(1),
        )
    };
    if ready {
        world.insert_resource(AnimatedPrims(animated));
        if let Some(mut stats) = world.get_resource_mut::<ProjectionStats>() {
            stats.initial_projection_ms = duration_ms;
            stats.initial_projection_prims = prims as u64;
        }
        bevy::log::info!(
            session = live.session_id(),
            prims,
            duration_ms = duration_ms.unwrap_or_default(),
            "progressive USD stage projection ready"
        );
    }
}
