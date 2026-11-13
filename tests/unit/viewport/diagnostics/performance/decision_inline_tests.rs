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
