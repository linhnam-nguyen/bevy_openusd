use std::fs;

use super::*;

#[test]
fn worker_returns_owned_inspection_for_a_valid_usd_source() {
    let directory = tempfile::tempdir().expect("inspection source directory");
    let source = directory.path().join("assembly.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n    defaultPrim = \"Root\"\n)\ndef Xform \"Root\" {}\n",
    )
    .expect("write inspection source");

    let progress = ProjectImportProgressStore::default();
    let queue = ProjectSceneInspectionQueue::with_progress(progress.clone());
    let result = queue.inspect("operation-1".to_owned(), 7, source);

    assert_eq!(result.operation_id, "operation-1");
    assert_eq!(result.generation, 7);
    assert_eq!(result.progress.phase, ProjectImportPhase::Inspecting);
    let inspection = result.inspection.expect("valid USD should inspect");
    assert!(!inspection.diagnostics.is_empty() || inspection.dependencies.is_empty());
    assert_eq!(
        progress.latest("operation-1", 7).unwrap().phase,
        ProjectImportPhase::Inspecting
    );
}
