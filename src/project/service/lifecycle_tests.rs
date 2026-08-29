use std::fs;

use project_protocol::{
    ProjectInspectionClassification, ProjectInspectionWarning, ProjectWriteError,
    ProjectWriteErrorCode,
};
use tempfile::tempdir;
use usd_git::GitRepository;
use usd_project::{ProjectManifestV1, ProjectRoot};

use super::*;
use crate::project::catalog::manifest_store::ManifestStore;
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
fn remove_project_unregisters_without_touching_the_repository() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let registry_path = directory.path().join("config/workspace.json");
    let mut service = ProjectApplicationService::open(&registry_path).unwrap();
    let summary = service.create_project(&parent, "Removable").unwrap();
    let project_root = parent.join("Removable");
    let marker = project_root.join("user.usda");
    fs::write(&marker, b"#usda 1.0\n").unwrap();

    service.remove_project(summary.id).unwrap();

    assert!(project_root.is_dir());
    assert_eq!(fs::read(&marker).unwrap(), b"#usda 1.0\n");
    assert!(
        WorkspaceRegistry::load(&registry_path)
            .unwrap()
            .get(summary.id)
            .is_none()
    );
    assert!(usd_git::Repository::open(&project_root).is_ok());
}

#[test]
fn delete_project_removes_registered_repository_and_survives_restart() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let registry_path = directory.path().join("config/workspace.json");
    let mut service = ProjectApplicationService::open(&registry_path).unwrap();
    let summary = service.create_project(&parent, "Deletable").unwrap();
    let project_root = parent.join("Deletable");

    service.delete_project(summary.id).unwrap();

    assert!(!project_root.exists());
    assert!(
        WorkspaceRegistry::load(&registry_path)
            .unwrap()
            .get(summary.id)
            .is_none()
    );
    assert!(
        ProjectApplicationService::open(&registry_path)
            .unwrap()
            .remove_project(summary.id)
            .is_err()
    );
}

#[test]
fn delete_project_rejects_manifest_identity_mismatch_without_touching_folder() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let project_root = parent.join("Mismatched");
    usd_git::Repository::init(&project_root).unwrap();
    let actual_id = usd_project::ProjectId::new_v4();
    let requested_id = usd_project::ProjectId::new_v4();
    let manifest = ProjectManifestV1::new(
        actual_id,
        "Mismatched",
        ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(&project_root, &manifest).unwrap();
    let marker = project_root.join("keep.usda");
    fs::write(&marker, b"keep").unwrap();
    let registry_path = directory.path().join("config/workspace.json");
    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry
        .register(requested_id, &project_root, None)
        .unwrap();
    let mut service = ProjectApplicationService::open(&registry_path).unwrap();

    assert!(matches!(
        service.delete_project(requested_id),
        Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::ProjectDeleteFailed
        })
    ));
    assert!(project_root.is_dir());
    assert_eq!(fs::read(&marker).unwrap(), b"keep");
    assert!(
        WorkspaceRegistry::load(&registry_path)
            .unwrap()
            .get(requested_id)
            .is_some()
    );
}

#[test]
fn delete_project_restores_folder_when_registry_persistence_fails() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let registry_path = directory.path().join("config/workspace.json");
    let mut service = ProjectApplicationService::open(&registry_path).unwrap();
    let summary = service.create_project(&parent, "Rollback").unwrap();
    let project_root = parent.join("Rollback");
    let registry_bytes = fs::read(&registry_path).unwrap();

    fs::remove_file(&registry_path).unwrap();
    fs::create_dir(&registry_path).unwrap();
    let result = service.delete_project(summary.id);

    assert!(matches!(
        result,
        Err(ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::ProjectDeleteFailed
        })
    ));
    assert!(project_root.is_dir());
    assert!(!parent.read_dir().unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("usdhub-delete")
    }));
    fs::remove_dir(&registry_path).unwrap();
    fs::write(&registry_path, registry_bytes).unwrap();
    assert!(
        WorkspaceRegistry::load(&registry_path)
            .unwrap()
            .get(summary.id)
            .is_some()
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
    let before = super::import_tests::snapshot(&project_root);
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
    assert_eq!(before, super::import_tests::snapshot(&project_root));
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
        2
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
    let before = super::import_tests::snapshot(&project_root);
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
    assert_eq!(before, super::import_tests::snapshot(&project_root));
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
    super::import_tests::run_git(&project_root, ["add", "."]);
    super::import_tests::run_git(&project_root, ["commit", "-m", "track derived state"]);

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
