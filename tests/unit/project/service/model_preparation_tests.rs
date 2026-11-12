use std::fs;

use tempfile::tempdir;

use super::*;

#[test]
fn bounded_worker_prepares_owned_model_metadata_without_exposing_source_path() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("asset.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Asset\"\n)\ndef Xform \"Asset\" (kind = \"component\") {}\n",
    )
    .unwrap();
    let progress = ProjectImportProgressStore::default();
    let queue = ProjectModelPreparationQueue::with_progress(progress.clone());
    let result = queue.prepare("operation-1".to_owned(), 4, source);

    assert_eq!(result.operation_id, "operation-1");
    assert_eq!(result.generation, 4);
    assert!(result.inspection.is_ok());
    assert!(queue.take_prepared("operation-1", 4).is_some());
    assert_eq!(
        progress.latest("operation-1", 4).unwrap().phase,
        ProjectImportPhase::Preparing
    );
}

#[test]
fn abandoned_prepared_models_are_evicted_after_the_bounded_retention_window() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("asset.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Asset\"\n)\ndef Xform \"Asset\" (kind = \"component\") {}\n",
    )
    .unwrap();
    let queue = ProjectModelPreparationQueue::default();

    for generation in 0..=PREPARATION_CAPACITY as u64 {
        let result = queue.prepare("eviction".to_owned(), generation, source.clone());
        assert!(result.inspection.is_ok());
    }

    assert!(queue.take_prepared("eviction", 0).is_none());
    assert!(
        queue
            .take_prepared("eviction", PREPARATION_CAPACITY as u64)
            .is_some()
    );
}
