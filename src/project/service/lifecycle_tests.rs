use std::{collections::BTreeMap, fs, path::Path};

use project_protocol::{
    ProjectInspectionWarning, ProjectWriteError, ProjectWriteErrorCode, ProjectWriteTarget,
};
use tempfile::tempdir;
use usd_git::GitRepository;

use super::*;
use crate::project::catalog::workspace_registry::WorkspaceRegistry;

#[test]
fn create_project_keeps_head_unborn_and_registers_last() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("keep.txt"), b"user data").unwrap();
    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(&registry_path).unwrap();

    let summary = service.create_project(&parent, "Created Project").unwrap();
    let project_root = parent.join("Created Project");
    let repository = usd_git::Repository::open(&project_root).unwrap();

    assert_eq!(summary.name, "Created Project");
    assert_eq!(
        repository.current_branch().unwrap().as_deref(),
        Some("main")
    );
    assert!(repository.head().unwrap().is_none());
    assert!(project_root.join(".git").is_dir());
    assert!(project_root.join(".usdhub/project.json").is_file());
    assert!(project_root.join(".usdhub/cache").is_dir());
    assert!(project_root.join(".usdhub/recovery").is_dir());
    assert_eq!(fs::read(parent.join("keep.txt")).unwrap(), b"user data");
    assert!(
        fs::read_to_string(project_root.join(".gitignore"))
            .unwrap()
            .contains(".usdhub/cache/")
    );
    assert_eq!(
        WorkspaceRegistry::load(registry_path)
            .unwrap()
            .get(summary.id)
            .unwrap()
            .repository_locator(),
        project_root
    );
}

#[test]
fn create_project_rejects_unsafe_names_without_touching_parent() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("keep.txt"), b"user data").unwrap();
    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(registry_path).unwrap();

    for name in ["", ".", "..", "nested/name", "nested\\name", "bad\0name"] {
        assert!(matches!(
            service.create_project(&parent, name),
            Err(ProjectWriteError::Invalid {
                code: ProjectWriteErrorCode::InvalidProjectName
            })
        ));
    }
    assert_eq!(fs::read(parent.join("keep.txt")).unwrap(), b"user data");
    assert_eq!(fs::read_dir(parent).unwrap().count(), 1);
}

#[test]
fn import_inspection_is_read_only_and_classifies_adoptable_git() {
    let directory = tempdir().unwrap();
    let project_root = directory.path().join("existing");
    usd_git::Repository::init(&project_root).unwrap();
    fs::write(project_root.join("user.usda"), b"#usda 1.0\n").unwrap();
    let before = snapshot(&project_root);
    let service = ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();

    let inspection = service.inspect_project(&project_root).unwrap();

    assert_eq!(
        inspection.classification,
        ProjectInspectionClassification::AdoptableGit
    );
    assert!(
        inspection
            .warnings
            .contains(&ProjectInspectionWarning::MissingLocalCacheRoots)
    );
    assert_eq!(before, snapshot(&project_root));
}

#[test]
fn native_project_with_deleted_local_state_remains_importable() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(&registry_path).unwrap();
    let summary = service.create_project(&parent, "Native").unwrap();
    let project_root = parent.join("Native");
    fs::remove_dir_all(project_root.join(".usdhub/cache")).unwrap();
    fs::remove_dir_all(project_root.join(".usdhub/recovery")).unwrap();

    let inspection = service.inspect_project(&project_root).unwrap();

    assert_eq!(
        inspection.classification,
        ProjectInspectionClassification::NativeUsdHub
    );
    assert!(
        inspection
            .warnings
            .contains(&ProjectInspectionWarning::MissingLocalCacheRoots)
    );
    assert_eq!(inspection.display_name, summary.name);
    assert_eq!(
        fs::read_dir(project_root.join(".usdhub")).unwrap().count(),
        1
    );

    service.import_project(&project_root, &inspection).unwrap();
    assert!(project_root.join(".usdhub/cache").is_dir());
    assert!(project_root.join(".usdhub/recovery").is_dir());
}

#[test]
fn confirmed_adoption_adds_metadata_without_rewriting_git_history() {
    let directory = tempdir().unwrap();
    let project_root = directory.path().join("adopted");
    usd_git::Repository::init(&project_root).unwrap();
    fs::write(project_root.join("user.usda"), b"#usda 1.0\n").unwrap();
    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(&registry_path).unwrap();
    let inspection = service.inspect_project(&project_root).unwrap();

    let summary = service.import_project(&project_root, &inspection).unwrap();
    let repository = usd_git::Repository::open(&project_root).unwrap();

    assert_eq!(
        inspection.classification,
        ProjectInspectionClassification::AdoptableGit
    );
    assert_eq!(summary.name, "adopted");
    assert!(repository.head().unwrap().is_none());
    assert!(project_root.join("user.usda").is_file());
    assert!(project_root.join(".usdhub/project.json").is_file());
    assert!(project_root.join(".usdhub/cache").is_dir());
    assert!(
        WorkspaceRegistry::load(registry_path)
            .unwrap()
            .get(summary.id)
            .is_some()
    );
}

#[test]
fn broad_ignore_conflict_is_reported_without_mutation() {
    let directory = tempdir().unwrap();
    let project_root = directory.path().join("conflict");
    usd_git::Repository::init(&project_root).unwrap();
    fs::write(project_root.join(".gitignore"), b".usdhub/\nkeep\n").unwrap();
    let before = snapshot(&project_root);
    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(&registry_path).unwrap();
    let inspection = service.inspect_project(&project_root).unwrap();

    assert!(
        inspection
            .warnings
            .contains(&ProjectInspectionWarning::BroadUsdHubIgnore)
    );
    assert!(matches!(
        service.import_project(&project_root, &inspection),
        Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::IgnoreConflict
        })
    ));
    assert_eq!(before, snapshot(&project_root));
}

#[test]
fn inspection_warns_when_derived_local_state_is_tracked() {
    let directory = tempdir().unwrap();
    let project_root = directory.path().join("tracked-derived");
    usd_git::Repository::init(&project_root).unwrap();
    fs::create_dir_all(project_root.join(".usdhub/cache")).unwrap();
    fs::create_dir_all(project_root.join(".usdhub/recovery")).unwrap();
    fs::write(project_root.join(".usdhub/cache/object"), b"cache").unwrap();
    fs::write(project_root.join(".usdhub/recovery/session"), b"recovery").unwrap();
    run_git(&project_root, ["add", "."]);
    run_git(&project_root, ["commit", "-m", "track derived state"]);

    let service = ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let inspection = service.inspect_project(&project_root).unwrap();

    assert!(
        inspection
            .warnings
            .contains(&ProjectInspectionWarning::TrackedDerivedLocalState)
    );
}

#[test]
fn adoption_failure_restores_a_pre_existing_gitignore_byte_for_byte() {
    let directory = tempdir().unwrap();
    let project_root = directory.path().join("rollback-ignore");
    usd_git::Repository::init(&project_root).unwrap();
    let original_ignore = b"user-rule/\n# preserve this exact file\n";
    fs::write(project_root.join(".gitignore"), original_ignore).unwrap();
    fs::create_dir_all(project_root.join(".usdhub")).unwrap();
    fs::write(project_root.join(".usdhub/cache"), b"user data").unwrap();

    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(registry_path).unwrap();
    let inspection = service.inspect_project(&project_root).unwrap();
    let result = service.import_project(&project_root, &inspection);

    assert!(matches!(
        result,
        Err(ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::FilesystemFailure
        })
    ));
    assert_eq!(
        fs::read(project_root.join(".gitignore")).unwrap(),
        original_ignore
    );
    assert!(!project_root.join(".usdhub/project.json").exists());
    assert_eq!(
        fs::read(project_root.join(".usdhub/cache")).unwrap(),
        b"user data"
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

#[test]
fn create_scene_promotes_an_empty_project_without_a_fake_placement() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(registry_path).unwrap();
    let project = service.create_project(&parent, "Scene Project").unwrap();

    let created = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Main Scene",
        )
        .unwrap();

    assert_eq!(created.placement_id, None);
    assert!(matches!(
        created.project.root,
        usd_project::ProjectRoot::Scene(scene_id) if scene_id == created.scene_id
    ));
    assert!(
        parent
            .join("Scene Project/.usdhub/scenes")
            .join(format!("{}.usda", created.scene_id))
            .is_file()
    );
    let manifest = ManifestStore::read_validated(&parent.join("Scene Project")).unwrap();
    assert_eq!(manifest.scenes().len(), 1);
    assert_eq!(manifest.raw().root, created.project.root);
}

#[test]
fn create_scene_adds_one_identity_preserving_child_placement() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(registry_path).unwrap();
    let project = service
        .create_project(&parent, "Nested Scene Project")
        .unwrap();
    let root = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Main Scene",
        )
        .unwrap();

    let child = service
        .create_scene(
            project.id,
            ProjectWriteTarget::Scene(root.scene_id),
            "Child Scene",
        )
        .unwrap();

    let project_root = parent.join("Nested Scene Project");
    let members = crate::project::scene::authoring::read_scene_members(
        &crate::project::scene::authoring::scene_path(&project_root, root.scene_id),
        root.scene_id,
    )
    .unwrap();
    assert_eq!(child.placement_id, members.first().map(|member| member.id));
    assert!(matches!(
        members.as_slice(),
        [usd_project::SceneMember {
            target: usd_project::SceneMemberTarget::Scene(target),
            name: Some(name),
            ..
        }] if *target == child.scene_id && name == "Child Scene"
    ));
    assert_eq!(
        ManifestStore::read_validated(&project_root)
            .unwrap()
            .scenes()
            .len(),
        2
    );
}

#[test]
fn create_scene_rejects_model_root_and_invalid_names_without_mutation() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(registry_path).unwrap();
    let project = service
        .create_project(&parent, "Guarded Scene Project")
        .unwrap();
    let project_root = parent.join("Guarded Scene Project");

    let before = ManifestStore::read_validated(&project_root)
        .unwrap()
        .raw()
        .clone();
    assert_eq!(
        service.create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "../escape",
        ),
        Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::InvalidSceneName
        })
    );
    assert_eq!(
        ManifestStore::read_validated(&project_root).unwrap().raw(),
        &before
    );

    let model_id = usd_project::ModelId::new_v4();
    let mut model_root = before;
    model_root.models.push(usd_project::ModelManifestEntry {
        id: model_id,
        source_kind: usd_project::ModelSourceKind::Usd,
        storage_key: usd_project::StorageKey::new("model").unwrap(),
    });
    model_root.root = usd_project::ProjectRoot::Model(model_id);
    ManifestStore::write_manifest_atomic(&project_root, &model_root).unwrap();
    assert_eq!(
        service.create_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            "Blocked Scene",
        ),
        Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::InvalidRootForComposition
        })
    );
}

fn run_git<const N: usize>(root: &Path, args: [&str; N]) {
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

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
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
