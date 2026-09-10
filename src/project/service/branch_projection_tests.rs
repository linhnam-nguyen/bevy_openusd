use std::{fs, path::Path, process::Command};

use project_protocol::ProjectWriteError;
use tempfile::tempdir;
use usd_git::GitRepository;
use usd_project::{ProjectManifestV1, ProjectRoot, SceneManifestEntry, StorageKey};

use super::{ManifestStore, ProjectApplicationService, WorkspaceRegistry};

#[test]
fn successful_branch_recovery_removes_unknown_previous_scene_cache_membership() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("project");
    std::fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init", "-b", "main"]);
    run_git(&repository, &["config", "user.name", "USDHub Test"]);
    run_git(&repository, &["config", "user.email", "test@usdhub.invalid"]);

    let project_id = usd_project::ProjectId::new_v4();
    let manifest = crate::project::scene::root::ensure_protected_root_scene_atomic(
        &repository,
        &usd_project::ProjectManifestV1::new(
            project_id,
            "Recovered Project",
            usd_project::ProjectRoot::Empty,
            Vec::new(),
            Vec::new(),
        ),
    )
    .unwrap();
    crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
        &repository,
        &manifest,
    )
    .unwrap();
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-m", "valid main"]);

    run_git(&repository, &["checkout", "-b", "broken"]);
    std::fs::remove_file(repository.join(".usdhub/project.json")).unwrap();
    run_git(&repository, &["add", "-A"]);
    run_git(&repository, &["commit", "-m", "invalid previous Project"]);
    run_git(&repository, &["checkout", "main"]);

    let registry_path = directory.path().join("workspace.json");
    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(project_id, &repository, None).unwrap();
    let mut service = ProjectApplicationService::open(registry_path).unwrap();
    run_git(&repository, &["checkout", "broken"]);

    let stale_scene = usd_project::SceneId::new_v4();
    let cache = crate::project::cache::SceneCacheStore::new(&repository);
    let config = crate::project::cache_hydration::default_project_cache_config_hash();
    cache.advance_generation(stale_scene, config).unwrap();
    let layout = crate::project::storage::ProjectStorageLayout::new(&repository);
    let objects = layout.scene_cache_objects_dir(stale_scene);
    std::fs::create_dir_all(&objects).unwrap();
    std::fs::write(objects.join("orphan.bin"), b"old branch").unwrap();
    let lookup = layout.project_cache_index_path();
    std::fs::create_dir_all(lookup.parent().unwrap()).unwrap();
    std::fs::write(&lookup, stale_scene.to_string()).unwrap();

    service.switch_branch(project_id, "main").unwrap();

    assert!(cache.load_descriptor(stale_scene).unwrap().is_none());
    assert!(!objects.exists());
    if lookup.exists() {
        let bytes = std::fs::read(&lookup).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(&stale_scene.to_string()));
    }
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
    let project_id = usd_project::ProjectId::new_v4();
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
    let root_scene = match manifest.root {
        ProjectRoot::Scene(scene_id) => scene_id,
        _ => panic!("protected root Scene expected"),
    };
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-m", "main Project"]);
    run_git(&repository, &["branch", "broken-feature"]);
    run_git(&repository, &["checkout", "broken-feature"]);
    fs::write(repository.join("project.json"), b"not a Project manifest").unwrap();
    run_git(&repository, &["add", "project.json"]);
    run_git(&repository, &["commit", "-m", "break Project metadata"]);
    run_git(&repository, &["checkout", "main"]);

    use crate::project::blob_store::BlobStore;
    let cache = crate::project::cache::SceneCacheStore::new(&repository);
    let config = crate::project::cache_hydration::default_project_cache_config_hash();
    cache.advance_generation(root_scene, config).unwrap();
    cache.object_store(root_scene).unwrap().put(b"main-only-cache").unwrap();
    let lookup = crate::project::storage::ProjectStorageLayout::new(&repository).project_cache_index_path();
    fs::create_dir_all(lookup.parent().unwrap()).unwrap();
    fs::write(&lookup, b"stale-main-lookup").unwrap();

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
    assert!(cache.load_descriptor(root_scene).unwrap().is_none());
    assert!(!crate::project::storage::ProjectStorageLayout::new(&repository)
        .scene_cache_objects_dir(root_scene).exists());
    assert!(!lookup.exists());

    service
        .switch_branch(project_id, "main")
        .expect("valid branch remains an explicit recovery path");
}

#[test]
fn branch_switch_succeeds_with_unavailable_worker_and_bounded_warm_pressure() {
    let directory = tempdir().unwrap();
    let repository = directory.path().join("project");
    fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init", "-b", "main"]);
    run_git(&repository, &["config", "user.name", "USDHub Test"]);
    run_git(
        &repository,
        &["config", "user.email", "test@usdhub.invalid"],
    );
    crate::project::storage::install_managed_ignore(&repository).unwrap();

    let project_id = usd_project::ProjectId::new_v4();
    let manifest = crate::project::scene::root::ensure_protected_root_scene_atomic(
        &repository,
        &ProjectManifestV1::new(
            project_id,
            "Branch Admission",
            ProjectRoot::Empty,
            Vec::new(),
            Vec::new(),
        ),
    )
    .unwrap();
    let root_scene = match manifest.root {
        ProjectRoot::Scene(scene_id) => scene_id,
        _ => panic!("protected root Scene expected"),
    };
    ManifestStore::write_manifest_atomic(&repository, &manifest).unwrap();
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-m", "main Project"]);

    run_git(&repository, &["checkout", "-b", "feature"]);
    let feature_scenes = (0..9)
        .map(|index| SceneManifestEntry {
            id: usd_project::SceneId::new_v4(),
            storage_key: StorageKey::new(format!("feature-scene-{index}"))
                .expect("feature Scene storage key"),
            display_name: format!("Feature Scene {index}"),
        })
        .collect::<Vec<_>>();
    let mut feature_manifest = ManifestStore::read_validated(&repository)
        .unwrap()
        .raw()
        .clone();
    feature_manifest.scenes.extend(feature_scenes.iter().cloned());
    ManifestStore::write_manifest_atomic(&repository, &feature_manifest).unwrap();
    for scene in &feature_scenes {
        crate::project::scene::authoring::author_scene_atomic(&repository, scene.id).unwrap();
    }
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-m", "feature Scenes"]);
    run_git(&repository, &["checkout", "main"]);

    let config = crate::project::cache_hydration::default_project_cache_config_hash();
    let store = crate::project::cache::SceneCacheStore::new(&repository);
    let root_generation = store.advance_generation(root_scene, config).unwrap();
    let registry_path = directory.path().join("workspace.json");
    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(project_id, &repository, None).unwrap();
    let mut service = ProjectApplicationService::open(registry_path).unwrap();
    service.cache_warm.shutdown_without_waiting();

    let response = service
        .switch_branch(project_id, "feature")
        .expect("worker absence and bounded advisory pressure are non-fatal");

    assert_eq!(response.repository.active_branch.as_deref(), Some("feature"));
    assert_eq!(
        store.load_descriptor(root_scene).unwrap().unwrap().generation,
        root_generation
    );
    for scene in &feature_scenes {
        assert_eq!(
            store.load_descriptor(scene.id).unwrap().unwrap().generation,
            1,
            "synchronous Scene boundary must advance before advisory admission"
        );
    }
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

    let project_id = usd_project::ProjectId::new_v4();
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
