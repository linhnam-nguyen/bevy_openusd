use super::*;
use project_protocol::ProjectImportPhase;

fn progress(
    operation_id: &str,
    generation: u64,
    phase: ProjectImportPhase,
) -> ProjectImportProgress {
    ProjectImportProgress {
        operation_id: operation_id.to_owned(),
        generation,
        phase,
    }
}

#[test]
fn latest_phase_is_coalesced_per_operation_generation() {
    let store = ProjectImportProgressStore::default();
    store.publish(progress("operation", 4, ProjectImportPhase::Queued));
    store.publish(progress("operation", 4, ProjectImportPhase::Preparing));
    assert_eq!(
        store.latest("operation", 4).unwrap().phase,
        ProjectImportPhase::Preparing
    );
}

#[test]
fn old_generations_are_bounded_out_of_the_status_store() {
    let store = ProjectImportProgressStore::default();
    for generation in 0..=PROGRESS_CAPACITY as u64 {
        store.publish(progress(
            "operation",
            generation,
            ProjectImportPhase::Queued,
        ));
    }
    assert!(store.latest("operation", 0).is_none());
    assert!(
        store
            .latest("operation", PROGRESS_CAPACITY as u64)
            .is_some()
    );
}
