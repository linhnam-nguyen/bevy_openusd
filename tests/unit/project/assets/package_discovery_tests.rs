use super::*;
use usd_project::ProjectId;

#[test]
fn empty_project_has_no_import_packages() {
    let root = tempfile::tempdir().expect("temporary Project root");
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Archive closure",
        ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    assert!(
        for_target(
            root.path(),
            &manifest,
            &ProjectStageTarget::ProjectRoot(ProjectRoot::Empty),
            &root.path().join("scene.usda"),
        )
        .expect("archive closure")
        .is_empty()
    );
}
