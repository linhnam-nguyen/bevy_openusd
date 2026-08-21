//! Pure decision helpers and boundary observability rules for grid, semantic sync, and data isolation.

use serde::{Deserialize, Serialize};

/// Decision outcome for semantic stage synchronization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SemanticSyncWorkAction {
    /// Same session, valid snapshot present, no pending batch: skip clone and extraction.
    NoWork,
    /// New session or missing snapshot: perform full initial extraction.
    InitialExtract,
    /// Same session with newer stage change batch: perform incremental update.
    IncrementalWork,
}

/// Pure decision helper for evaluating semantic stage synchronization necessity.
pub struct SemanticDecisionHelper;

impl SemanticDecisionHelper {
    /// Pure decision truth table function for semantic sync workload.
    pub fn decide(
        has_live_stage: bool,
        current_session: Option<u64>,
        stage_session: u64,
        has_snapshot: bool,
        has_pending_batch: bool,
    ) -> SemanticSyncWorkAction {
        if !has_live_stage {
            return SemanticSyncWorkAction::NoWork;
        }

        let is_same_session = current_session == Some(stage_session);

        if is_same_session && has_snapshot {
            if has_pending_batch {
                SemanticSyncWorkAction::IncrementalWork
            } else {
                SemanticSyncWorkAction::NoWork
            }
        } else {
            SemanticSyncWorkAction::InitialExtract
        }
    }
}

/// Pure decision helper for ground grid field mutation suppression.
pub struct GroundGridDecisionHelper;

impl GroundGridDecisionHelper {
    pub const DEFAULT_TOLERANCE: f32 = 1e-4;

    /// Checks if a float field has changed beyond numerical noise tolerance.
    pub fn field_changed(current: f32, desired: f32, tolerance: f32) -> bool {
        (current - desired).abs() > tolerance
    }

    /// Checks if an optional float field has changed beyond numerical noise.
    pub fn optional_field_changed(
        current: Option<f32>,
        desired: Option<f32>,
        tolerance: f32,
    ) -> bool {
        match (current, desired) {
            (None, None) => false,
            (Some(current), Some(desired)) => Self::field_changed(current, desired, tolerance),
            _ => true,
        }
    }

    /// Determines if ground_y requires an in-place mutation.
    pub fn needs_y_update(current: f32, desired: f32, tolerance: f32) -> bool {
        Self::field_changed(current, desired, tolerance)
    }

    /// Determines if coverage_radius requires an in-place mutation.
    pub fn needs_radius_update(current: f32, desired: f32, tolerance: f32) -> bool {
        Self::field_changed(current, desired, tolerance)
    }

    /// Determines if any grid runtime property requires mutation.
    pub fn needs_mutation(
        current_y: f32,
        desired_y: f32,
        current_radius: f32,
        desired_radius: f32,
        tolerance: f32,
    ) -> bool {
        Self::needs_y_update(current_y, desired_y, tolerance)
            || Self::needs_radius_update(current_radius, desired_radius, tolerance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_decision_truth_table_invariants() {
        // 1. Same session + snapshot + no batch => NoWork
        assert_eq!(
            SemanticDecisionHelper::decide(true, Some(1), 1, true, false),
            SemanticSyncWorkAction::NoWork
        );

        // 2. Same session + snapshot + pending batch => IncrementalWork
        assert_eq!(
            SemanticDecisionHelper::decide(true, Some(1), 1, true, true),
            SemanticSyncWorkAction::IncrementalWork
        );

        // 3. Same session + no snapshot => InitialExtract
        assert_eq!(
            SemanticDecisionHelper::decide(true, Some(1), 1, false, false),
            SemanticSyncWorkAction::InitialExtract
        );

        // 4. New session => InitialExtract
        assert_eq!(
            SemanticDecisionHelper::decide(true, Some(1), 2, true, false),
            SemanticSyncWorkAction::InitialExtract
        );
        assert_eq!(
            SemanticDecisionHelper::decide(true, None, 1, false, false),
            SemanticSyncWorkAction::InitialExtract
        );

        // 5. No LiveStage => NoWork
        assert_eq!(
            SemanticDecisionHelper::decide(false, Some(1), 1, true, true),
            SemanticSyncWorkAction::NoWork
        );
    }

    #[test]
    fn ground_grid_decision_tolerance_invariants() {
        let tol = GroundGridDecisionHelper::DEFAULT_TOLERANCE;

        // Identical values
        assert!(!GroundGridDecisionHelper::needs_y_update(0.0, 0.0, tol));
        assert!(!GroundGridDecisionHelper::needs_radius_update(
            100.0, 100.0, tol
        ));
        assert!(!GroundGridDecisionHelper::needs_mutation(
            0.0, 0.0, 100.0, 100.0, tol
        ));

        // Sub-tolerance numerical jitter
        assert!(!GroundGridDecisionHelper::needs_y_update(0.0, 1e-6, tol));
        assert!(!GroundGridDecisionHelper::needs_radius_update(
            100.0,
            100.0 + 1e-6,
            tol
        ));
        assert!(!GroundGridDecisionHelper::needs_mutation(
            0.0,
            1e-6,
            100.0,
            100.0 + 1e-6,
            tol
        ));

        // Material change in ground_y
        assert!(GroundGridDecisionHelper::needs_y_update(0.0, 0.5, tol));
        assert!(GroundGridDecisionHelper::needs_mutation(
            0.0, 0.5, 100.0, 100.0, tol
        ));

        // Material change in coverage_radius
        assert!(GroundGridDecisionHelper::needs_radius_update(
            100.0, 150.0, tol
        ));
        assert!(GroundGridDecisionHelper::needs_mutation(
            0.0, 0.0, 100.0, 150.0, tol
        ));

        // Optional ground reference changes only when presence or value changes materially.
        assert!(!GroundGridDecisionHelper::optional_field_changed(
            None, None, tol
        ));
        assert!(GroundGridDecisionHelper::optional_field_changed(
            None,
            Some(0.0),
            tol
        ));
        assert!(GroundGridDecisionHelper::optional_field_changed(
            Some(0.0),
            None,
            tol
        ));
        assert!(!GroundGridDecisionHelper::optional_field_changed(
            Some(0.0),
            Some(1e-6),
            tol
        ));
        assert!(GroundGridDecisionHelper::optional_field_changed(
            Some(0.0),
            Some(0.5),
            tol
        ));
    }
}
