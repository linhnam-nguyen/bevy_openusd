use std::fs;

use super::*;
use crate::project::scene::inspection::inspect_composition;
use project_protocol::{PlacementSpec, ProjectWriteTarget};
use tempfile::tempdir;

#[test]
fn adoption_reports_source_inspection_failures_with_a_typed_error() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Project").unwrap();
    let source = directory.path().join("assembly.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\") {}\n",
    )
    .unwrap();
    let inspection = inspect_composition(&source).unwrap();
    fs::remove_file(&source).unwrap();

    let error = service
        .adopt_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            &source,
            &inspection,
            "Assembly".to_owned(),
            "typed-source-error".to_owned(),
            1,
            PlacementSpec::Default,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        project_protocol::ProjectWriteError::Failed {
            code: project_protocol::ProjectWriteErrorCode::SourceChanged
        }
    ));
}

#[test]
fn adoption_rejects_a_changed_source_with_a_distinct_typed_error() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Project").unwrap();
    let source = directory.path().join("assembly.usda");
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\") {}\n",
    )
    .unwrap();
    let inspection = inspect_composition(&source).unwrap();
    fs::write(
        &source,
        "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n metersPerUnit = 1\n)\ndef Xform \"Assembly\" (kind = \"assembly\") {}\n",
    )
    .unwrap();

    let error = service
        .adopt_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            &source,
            &inspection,
            "Assembly".to_owned(),
            "typed-source-changed".to_owned(),
            1,
            PlacementSpec::Default,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        project_protocol::ProjectWriteError::Failed {
            code: project_protocol::ProjectWriteErrorCode::SourceChanged
        }
    ));
}

#[test]
fn adoption_accepts_valid_source_with_advisory_classification() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Project").unwrap();
    let source = directory.path().join("not-a-scene.usda");
    fs::write(&source, "#usda 1.0\ndef Xform \"World\" {}\n").unwrap();
    let inspection = inspect_composition(&source).unwrap();

    assert_eq!(
        inspection.classification,
        usd_project::CompositionClassification::Ambiguous
    );
    let response = service
        .adopt_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            &source,
            &inspection,
            "World".to_owned(),
            "typed-classification-error".to_owned(),
            1,
            PlacementSpec::Default,
        )
        .expect("valid USD remains importable despite advisory classification");
    assert_eq!(response.project.counts.scenes, 2);
    assert!(response.placement_id.is_some());
}
