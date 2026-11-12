use std::{collections::BTreeMap, fs, path::Path};

use tempfile::tempdir;
use usd_project::{ProjectManifestV1, ProjectRoot};

use super::*;
use crate::project::catalog::{
    manifest_store::ManifestStore, workspace_registry::WorkspaceRegistry,
};

#[test]
fn opening_the_workspace_migrates_a_legacy_empty_project_root() {
    let directory = tempdir().unwrap();
    let project_root = directory.path().join("legacy");
    usd_git::Repository::init(&project_root).unwrap();
    let project_id = usd_project::ProjectId::new_v4();
    let manifest = ProjectManifestV1::new(
        project_id,
        "Legacy Project",
        ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(&project_root, &manifest).unwrap();

    let registry_path = directory.path().join("workspace.json");
    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(project_id, &project_root, None).unwrap();

    let _service = ProjectApplicationService::open(&registry_path).unwrap();
    let migrated = ManifestStore::read_validated(&project_root).unwrap();
    let ProjectRoot::Scene(root_scene_id) = migrated.raw().root else {
        panic!("legacy Project must migrate to a protected Root Scene");
    };
    assert!(
        crate::project::scene::root::is_protected_root_scene(
            &crate::project::scene::authoring::scene_path(&project_root, root_scene_id),
            root_scene_id,
        )
        .unwrap()
    );
}

#[test]
fn tracked_derived_state_changes_invalidate_an_old_import_inspection() {
    let directory = tempdir().unwrap();
    let project_root = directory.path().join("stale-tracked-derived");
    usd_git::Repository::init(&project_root).unwrap();
    let service = ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let inspection = service.inspect_project(&project_root).unwrap();

    fs::create_dir_all(project_root.join(".usdhub/cache")).unwrap();
    fs::write(project_root.join(".usdhub/cache/object"), b"cache").unwrap();
    run_git(&project_root, ["add", "."]);
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();

    assert!(matches!(
        service.import_project(&project_root, &inspection),
        Err(ProjectWriteError::ConcurrentChange)
    ));
}

pub(super) fn run_git<const N: usize>(root: &Path, args: [&str; N]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(super) fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, current: &Path, output: &mut BTreeMap<String, Vec<u8>>) {
        let mut entries = fs::read_dir(current)
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if path.is_dir() {
                visit(root, &path, output);
            } else {
                output.insert(relative, fs::read(path).unwrap());
            }
        }
    }

    let mut output = BTreeMap::new();
    visit(root, root, &mut output);
    output
}
