use bevy::prelude::*;
use std::collections::HashSet;
use std::time::{Duration, Instant};

use super::projection_plan::ProjectionPlan;

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
    pub(super) generation: u64,
    pub(super) session_id: Option<u64>,
    pub(super) readiness: ProjectionReadiness,
    pub(super) completed: usize,
    pub(super) total: usize,
    pub(super) plan: Option<ProjectionPlan>,
    pub(super) next_index: usize,
    pub(super) entities: Vec<Option<Entity>>,
    pub(super) animated: HashSet<String>,
    pub(super) started_at: Option<Instant>,
    pub(super) planning_ms: Option<f64>,
    pub(super) planning_updates: u64,
    pub(super) planning_work_items: u64,
    pub(super) first_projected_prim_ms: Option<f64>,
    pub(super) first_mesh_ms: Option<f64>,
    pub(super) restart_requested: bool,
    pub(super) last_error: Option<String>,
    pub(super) cancelled_generations: u64,
    pub(super) plan_builds: u64,
    pub(super) resident_short_circuits: u64,
    pub(super) resident_validation_requested: bool,
}

impl ProgressiveProjectionState {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn session_id(&self) -> Option<u64> {
        self.session_id
    }
    pub fn readiness(&self) -> ProjectionReadiness {
        self.readiness
    }
    pub fn completed(&self) -> usize {
        self.completed
    }
    pub fn total(&self) -> usize {
        self.total
    }

    /// Progress is reported after planning establishes the final denominator.
    pub fn progress(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.completed as f32 / self.total as f32
        }
    }

    pub fn plan(&self) -> Option<&ProjectionPlan> {
        self.plan.as_ref()
    }
    pub fn planning_ms(&self) -> Option<f64> {
        self.planning_ms
    }
    pub fn planning_updates(&self) -> u64 {
        self.planning_updates
    }
    pub fn planning_work_items(&self) -> u64 {
        self.planning_work_items
    }
    pub fn first_projected_prim_ms(&self) -> Option<f64> {
        self.first_projected_prim_ms
    }
    pub fn first_mesh_ms(&self) -> Option<f64> {
        self.first_mesh_ms
    }
    pub fn cancelled_generations(&self) -> u64 {
        self.cancelled_generations
    }
    pub fn plan_builds(&self) -> u64 {
        self.plan_builds
    }
    pub fn resident_short_circuits(&self) -> u64 {
        self.resident_short_circuits
    }

    /// Request one validation of resident mesh/material handles on the next
    /// ready-state update. The normal idle path remains constant-time.
    pub fn invalidate_resident_cache(&mut self) {
        self.resident_validation_requested = true;
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Cancel the active generation and prioritize a fresh current-stage plan.
    pub fn cancel(&mut self) {
        if matches!(
            self.readiness,
            ProjectionReadiness::Planning | ProjectionReadiness::Projecting
        ) {
            self.readiness = ProjectionReadiness::Cancelled;
            self.restart_requested = true;
            self.cancelled_generations += 1;
        }
    }
}
