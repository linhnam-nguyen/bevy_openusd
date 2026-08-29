use std::{fs, path::Path, process::Command};

use openusd::usd::Stage;
use project_protocol::{ProjectWriteError, ProjectWriteErrorCode};
use tempfile::tempdir;
use usd_git::GitRepository;
use usd_project::{
    ProjectId, ProjectManifestV1, ProjectRoot, SceneManifestEntry, SceneMemberTarget, StorageKey,
};

use super::{ManifestStore, ProjectApplicationService, WorkspaceRegistry};

#[test]
fn service_switches_a_clean_registered_repository_and_rejects_dirty_work() {
    let directory = tempdir().unwrap();
    let repository = directory.path().join("project");
    fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init", "-b", "main"]);
    run_git(&repository, &["config", "user.name", "USDHub Test"]);
    run_git(
        &repository,
        &["config", "user.email", "test@usdhub.invalid"],
    );

    let project_id = ProjectId::new_v4();
    let base_manifest = ProjectManifestV1::new(
        project_id,
        "Branch Project",
        ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    let manifest = crate::project::scene::root::ensure_protected_root_scene_atomic(
        &repository,
        &base_manifest,
    )
    .unwrap();
    ManifestStore::write_manifest_atomic(&repository, &manifest).unwrap();
    fs::write(repository.join("branch.txt"), b"main").unwrap();
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-m", "main Project"]);
    run_git(&repository, &["branch", "feature"]);
    run_git(&repository, &["checkout", "feature"]);
    fs::write(repository.join("branch.txt"), b"feature").unwrap();
    run_git(&repository, &["add", "branch.txt"]);
    run_git(&repository, &["commit", "-m", "feature Project"]);
    run_git(&repository, &["checkout", "main"]);

    let registry_path = directory.path().join("workspace.json");
    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(project_id, &repository, None).unwrap();
    let mut service = ProjectApplicationService::open(registry_path).unwrap();

    let response = service
        .switch_branch(project_id, "feature")
        .expect("clean repository switches");
    assert_eq!(
        response.repository.active_branch.as_deref(),
        Some("feature")
    );
    assert_eq!(fs::read(repository.join("branch.txt")).unwrap(), b"feature");
    assert_eq!(
        usd_git::Repository::open(&repository)
            .unwrap()
            .current_branch()
            .unwrap()
            .as_deref(),
        Some("feature")
    );

    fs::write(repository.join("branch.txt"), b"local edit").unwrap();
    assert!(matches!(
        service.switch_branch(project_id, "main"),
        Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::DirtyWorkingTree
        })
    ));
    assert_eq!(
        fs::read(repository.join("branch.txt")).unwrap(),
        b"local edit"
    );
}

#[test]
fn switching_to_a_legacy_scene_branch_migrates_and_composes_content() {
    let directory = tempdir().unwrap();
    let repository = directory.path().join("project");
    fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init", "-b", "main"]);
    run_git(&repository, &["config", "user.name", "USDHub Test"]);
    run_git(
        &repository,
        &["config", "user.email", "test@usdhub.invalid"],
    );

    let project_id = ProjectId::new_v4();
    let scene_id = usd_project::SceneId::new_v4();
    let legacy_manifest = ProjectManifestV1::new(
        project_id,
        "Branch Project",
        ProjectRoot::Scene(scene_id),
        vec![SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("Legacy Scene").unwrap(),
            display_name: "Legacy Scene".to_owned(),
        }],
        Vec::new(),
    );
    crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
        &repository,
        &legacy_manifest,
    )
    .unwrap();
    write_legacy_scene_layer(&repository, scene_id);
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-m", "legacy Project"]);
    run_git(&repository, &["branch", "legacy-scene"]);

    let migrated_main = crate::project::scene::root::ensure_protected_root_scene_atomic(
        &repository,
        &legacy_manifest,
    )
    .unwrap();
    assert!(matches!(migrated_main.root, ProjectRoot::Scene(root) if root != scene_id));
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-m", "protected main Project"]);

    let registry_path = directory.path().join("workspace.json");
    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(project_id, &repository, None).unwrap();
    let mut service = ProjectApplicationService::open(registry_path).unwrap();

    let response = service
        .switch_branch(project_id, "legacy-scene")
        .expect("legacy branch migration should succeed");
    assert_eq!(
        response.repository.active_branch.as_deref(),
        Some("legacy-scene")
    );

    let migrated = ManifestStore::read_validated(&repository).unwrap();
    let ProjectRoot::Scene(root_id) = migrated.raw().root else {
        panic!("legacy branch must migrate to a protected Root Scene");
    };
    let root_path = crate::project::scene::authoring::scene_path(&repository, root_id);
    let member = crate::project::scene::authoring::read_scene_members(&root_path, root_id)
        .unwrap()
        .into_iter()
        .find(|member| member.target == SceneMemberTarget::Scene(scene_id))
        .expect("legacy branch Scene placement");
    let root_stage = Stage::open(root_path.to_string_lossy().as_ref()).unwrap();
    let member_path = crate::project::scene::authoring::scene_member_path(member.id);
    assert!(
        root_stage
            .prim(member_path.as_str())
            .child_names()
            .unwrap()
            .iter()
            .any(|name| name.as_str() == "LegacyBranch")
    );
}

#[test]
fn service_branch_switch_reports_typed_invalid_and_missing_branch_failures() {
    let directory = tempdir().unwrap();
    let repository = directory.path().join("project");
    fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init", "-b", "main"]);
    run_git(&repository, &["config", "user.name", "USDHub Test"]);
    run_git(
        &repository,
        &["config", "user.email", "test@usdhub.invalid"],
    );

    let project_id = ProjectId::new_v4();
    let base_manifest = ProjectManifestV1::new(
        project_id,
        "Branch Project",
        ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    let manifest = crate::project::scene::root::ensure_protected_root_scene_atomic(
        &repository,
        &base_manifest,
    )
    .unwrap();
    ManifestStore::write_manifest_atomic(&repository, &manifest).unwrap();
    fs::write(repository.join("branch.txt"), b"main").unwrap();
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-m", "main Project"]);

    let registry_path = directory.path().join("workspace.json");
    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(project_id, &repository, None).unwrap();
    let mut service = ProjectApplicationService::open(registry_path).unwrap();

    assert!(matches!(
        service.switch_branch(project_id, "../unsafe"),
        Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::InvalidBranchName
        })
    ));
    assert!(matches!(
        service.switch_branch(project_id, "missing"),
        Err(ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::BranchNotFound
        })
    ));
}

#[test]
fn invalid_target_branch_reports_repository_truth_after_checkout() {
    let directory = tempdir().unwrap();
    let repository = directory.path().join("project");
    fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init", "-b", "main"]);
    run_git(&repository, &["config", "user.name", "USDHub Test"]);
    run_git(
        &repository,
        &["config", "user.email", "test@usdhub.invalid"],
    );

    let project_id = ProjectId::new_v4();
    let base_manifest = ProjectManifestV1::new(
        project_id,
        "Branch Project",
        ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    let manifest = crate::project::scene::root::ensure_protected_root_scene_atomic(
        &repository,
        &base_manifest,
    )
    .unwrap();
    ManifestStore::write_manifest_atomic(&repository, &manifest).unwrap();
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-m", "main Project"]);
    run_git(&repository, &["branch", "broken-feature"]);
    run_git(&repository, &["checkout", "broken-feature"]);
    fs::write(
        repository.join(".usdhub/project.json"),
        b"not a Project manifest",
    )
    .unwrap();
    run_git(&repository, &["add", ".usdhub/project.json"]);
    run_git(&repository, &["commit", "-m", "break Project metadata"]);
    run_git(&repository, &["checkout", "main"]);

    let registry_path = directory.path().join("workspace.json");
    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(project_id, &repository, None).unwrap();
    let mut service = ProjectApplicationService::open(registry_path).unwrap();

    let error = service
        .switch_branch(project_id, "broken-feature")
        .expect_err("invalid target metadata must fail after checkout");
    let ProjectWriteError::BranchProjectInvalid { repository: truth } = error else {
        panic!("expected repository truth with BranchProjectInvalid");
    };
    assert_eq!(truth.active_branch.as_deref(), Some("broken-feature"));
    assert_eq!(
        usd_git::Repository::open(&repository)
            .unwrap()
            .current_branch()
            .unwrap()
            .as_deref(),
        Some("broken-feature")
    );

    service
        .switch_branch(project_id, "main")
        .expect("valid branch remains an explicit recovery path");
}

#[test]
fn invalid_target_scene_projection_reports_repository_truth_after_checkout() {
    let directory = tempdir().unwrap();
    let repository = directory.path().join("project");
    fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init", "-b", "main"]);
    run_git(&repository, &["config", "user.name", "USDHub Test"]);
    run_git(
        &repository,
        &["config", "user.email", "test@usdhub.invalid"],
    );

    let project_id = ProjectId::new_v4();
    let scene_id = usd_project::SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        project_id,
        "Branch Project",
        ProjectRoot::Scene(scene_id),
        vec![SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("root-scene").unwrap(),
            display_name: "Root Scene".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(&repository, &manifest).unwrap();
    crate::project::scene::authoring::author_scene_atomic_with_graph_and_protection(
        &repository,
        scene_id,
        &usd_project::SceneCompositionGraph::default(),
        &[],
        true,
    )
    .unwrap();
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-m", "main Project"]);
    run_git(&repository, &["branch", "broken-scene"]);
    run_git(&repository, &["checkout", "broken-scene"]);
    fs::write(
        crate::project::scene::authoring::scene_path(&repository, scene_id),
        b"not a Project Scene",
    )
    .unwrap();
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-m", "break Scene projection"]);
    run_git(&repository, &["checkout", "main"]);

    let registry_path = directory.path().join("workspace.json");
    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(project_id, &repository, None).unwrap();
    let mut service = ProjectApplicationService::open(registry_path).unwrap();

    let error = service
        .switch_branch(project_id, "broken-scene")
        .expect_err("invalid Scene projection must fail after checkout");
    let ProjectWriteError::BranchProjectInvalid { repository: truth } = error else {
        panic!("expected repository truth with BranchProjectInvalid");
    };
    assert_eq!(truth.active_branch.as_deref(), Some("broken-scene"));
    assert_eq!(
        usd_git::Repository::open(&repository)
            .unwrap()
            .current_branch()
            .unwrap()
            .as_deref(),
        Some("broken-scene")
    );

    service
        .switch_branch(project_id, "main")
        .expect("valid branch remains an explicit recovery path");
}

fn run_git(directory: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run git command");
    assert!(
        output.status.success(),
        "git command failed: {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_legacy_scene_layer(repository: &Path, scene_id: usd_project::SceneId) {
    let path = crate::project::scene::authoring::scene_path(repository, scene_id);
    crate::project::scene::authoring::author_scene_atomic(repository, scene_id).unwrap();
    let stage = Stage::open(path.to_string_lossy().as_ref()).unwrap();
    stage
        .define_prim("/SceneRoot/LegacyBranch/Content")
        .unwrap()
        .set_type_name("Xform")
        .unwrap();
    stage.set_default_prim("SceneRoot").unwrap();
    stage
        .root_layer()
        .export(path.to_string_lossy().as_ref())
        .unwrap();
}
