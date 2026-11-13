use bevy::prelude::*;

use super::progressive_state::{ProgressiveProjectionState, ProjectionReadiness};
use super::stage::LiveStage;

/// Product-facing milestones for opening a Project root through the existing
/// stage and projection pipeline.
///
/// This is an observation of renderer-owned state, not another projection
/// state machine. `ProjectReady` is emitted when a new live stage session is
/// admitted; the later milestones are derived from the existing
/// [`ProgressiveProjectionState`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectOpenReadiness {
    ProjectReady,
    StageOpened,
    FirstGeometry,
    ProjectionComplete,
}

impl ProjectOpenReadiness {
    fn rank(self) -> u8 {
        match self {
            Self::ProjectReady => 0,
            Self::StageOpened => 1,
            Self::FirstGeometry => 2,
            Self::ProjectionComplete => 3,
        }
    }
}

/// Read-only lifecycle observation for the current Project stage session.
///
/// The session identity and projection generation are retained so a delayed
/// completion from a replaced stage cannot advance the new Project session.
/// This resource does not own projection work; [`LiveStagePlugin`] remains the
/// sole owner of the projection scheduler.
#[derive(Resource, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProjectOpenReadinessState {
    stage_session_id: Option<u64>,
    activation_generation: u64,
    projection_generation: u64,
    readiness: Option<ProjectOpenReadiness>,
}

impl ProjectOpenReadinessState {
    pub fn stage_session_id(&self) -> Option<u64> {
        self.stage_session_id
    }

    /// Monotonic local generation for admitted stage sessions.
    pub fn activation_generation(&self) -> u64 {
        self.activation_generation
    }

    pub fn projection_generation(&self) -> u64 {
        self.projection_generation
    }

    pub fn readiness(&self) -> Option<ProjectOpenReadiness> {
        self.readiness
    }

    /// Observe the current renderer-owned stage/projection state.
    ///
    /// A new `stage_session_id` starts a new activation generation. Once a
    /// session has been superseded, observations from its old identity are
    /// ignored. Within the current session, readiness is monotonic so a
    /// delayed lower milestone cannot regress the open result.
    pub fn observe(
        &mut self,
        stage_session_id: u64,
        projection_generation: u64,
        projection_readiness: ProjectionReadiness,
        first_geometry: bool,
    ) -> bool {
        if self
            .stage_session_id
            .is_some_and(|current| stage_session_id < current)
        {
            return false;
        }
        if self.stage_session_id != Some(stage_session_id) {
            self.stage_session_id = Some(stage_session_id);
            self.activation_generation = self.activation_generation.saturating_add(1);
            self.projection_generation = projection_generation;
            self.readiness = Some(ProjectOpenReadiness::ProjectReady);
        } else if projection_generation < self.projection_generation {
            return false;
        } else {
            self.projection_generation = projection_generation;
        }

        let observed = if projection_readiness == ProjectionReadiness::Ready {
            ProjectOpenReadiness::ProjectionComplete
        } else if first_geometry {
            ProjectOpenReadiness::FirstGeometry
        } else {
            ProjectOpenReadiness::StageOpened
        };
        let current = self
            .readiness
            .expect("a stage session always starts at ProjectReady");
        if observed.rank() > current.rank() {
            self.readiness = Some(observed);
            true
        } else {
            false
        }
    }
}

/// Projects the current `LiveStage` and progressive projection state into the
/// lifecycle observation. All scheduling and geometry work remains owned by
/// `LiveStagePlugin` and `ProgressiveProjectionState`.
pub(super) fn observe_project_open_readiness(world: &mut World) {
    let Some((session_id, projection_generation, projection_readiness, first_geometry)) = world
        .get_non_send::<LiveStage>()
        .map(|live| live.session_id())
        .and_then(|session_id| {
            world
                .get_resource::<ProgressiveProjectionState>()
                .map(|state| {
                    (
                        session_id,
                        state.generation(),
                        state.readiness(),
                        state.first_mesh_ms().is_some(),
                    )
                })
        })
    else {
        return;
    };

    world.resource_mut::<ProjectOpenReadinessState>().observe(
        session_id,
        projection_generation,
        projection_readiness,
        first_geometry,
    );
}

#[cfg(test)]
#[path = "../../tests/unit/live/project_readiness_tests.rs"]
mod tests;
