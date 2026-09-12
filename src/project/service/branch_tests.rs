use std::{fs, path::Path, process::Command};

use openusd::usd::Stage;
use project_protocol::{ProjectWriteError, ProjectWriteErrorCode};
use tempfile::tempdir;
use usd_git::GitRepository;
use usd_project::{
    ProjectId, ProjectManifestV1, ProjectRoot, SceneManifestEntry, SceneMemberTarget, StorageKey,
};

use super::{ManifestStore, ProjectApplicationService, WorkspaceRegistry};

#[path = "branch_repository_truth_tests.rs"]
mod repository_truth;

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
    crate::project::storage::install_managed_ignore(&repository).unwrap();
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

    repository_truth::assert_managed_runtime_is_ignored_but_project_metadata_is_dirty(
        &repository,
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
fn branch_switch_removes_old_only_scene_v3_state_and_lookup_membership() {
    use crate::project::blob_store::BlobStore;

    let directory = tempdir().unwrap();
    let repository = directory.path().join("project");
    fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init", "-b", "main"]);
    run_git(&repository, &["config", "user.name", "USDHub Test"]);
    run_git(&repository, &["config", "user.email", "test@usdhub.invalid"]);
    let project_id = ProjectId::new_v4();
    let extra_scene = usd_project::SceneId::new_v4();
    let mut manifest = crate::project::scene::root::ensure_protected_root_scene_atomic(
        &repository,
        &ProjectManifestV1::new(project_id, "Branch Cache", ProjectRoot::Empty, Vec::new(), Vec::new()),
    ).unwrap();
    manifest.scenes.push(SceneManifestEntry {
        id: extra_scene,
        storage_key: StorageKey::new("extra").unwrap(),
        display_name: "Extra".to_owned(),
    });
    ManifestStore::write_manifest_atomic(&repository, &manifest).unwrap();
    crate::project::storage::install_managed_ignore(&repository).unwrap();
    let extra_scene_path = crate::project::scene::authoring::author_scene_atomic(&repository, extra_scene).unwrap();
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-m", "main with extra scene"]);
    run_git(&repository, &["checkout", "-b", "reduced"]);
    let mut reduced = ManifestStore::read_validated(&repository).unwrap().raw().clone();
    reduced.scenes.retain(|scene| scene.id != extra_scene);
    ManifestStore::write_manifest_atomic(&repository, &reduced).unwrap();
    fs::remove_file(extra_scene_path).unwrap();
    run_git(&repository, &["add", "-A"]);
    run_git(&repository, &["commit", "-m", "remove extra scene"]);
    run_git(&repository, &["checkout", "main"]);

    let config = crate::project::cache_hydration::default_project_cache_config_hash();
    let store = crate::project::cache::SceneCacheStore::new(&repository);
    let root_scene = match manifest.root {
        ProjectRoot::Scene(scene_id) => scene_id,
        _ => panic!("test Project has a root Scene"),
    };
    let root_generation = store.advance_generation(root_scene, config).unwrap();
    let generation = store.advance_generation(extra_scene, config).unwrap();
    let mut descriptor = crate::project::cache_contract::SceneCacheDescriptorV3::invalidated(extra_scene, generation, config);
    descriptor.state = crate::project::cache_contract::SceneCacheState::Partial;
    crate::project::cache_warm_runtime::build_and_publish_scene_cache_generation(&repository, &descriptor).unwrap();
    store.object_store(extra_scene).unwrap().put(b"old-branch-only").unwrap();

    let registry_path = directory.path().join("workspace.json");
    let mut registry = WorkspaceRegistry::load(&registry_path).unwrap();
    registry.register(project_id, &repository, None).unwrap();
    let mut service = ProjectApplicationService::open(registry_path).unwrap();
    service.switch_branch(project_id, "reduced").unwrap();

    assert!(store.load_descriptor(extra_scene).unwrap().is_none());
    assert_eq!(store.load_descriptor(root_scene).unwrap().unwrap().generation, root_generation);
    assert!(!crate::project::storage::ProjectStorageLayout::new(&repository).scene_cache_objects_dir(extra_scene).exists());
    let lookup = fs::read(crate::project::storage::ProjectStorageLayout::new(&repository).project_cache_index_path()).unwrap();
    assert!(!String::from_utf8_lossy(&lookup).contains(&extra_scene.to_string()));
}

#[test]
fn branch_a_b_a_replaces_names_and_revision_truth_without_leaking_old_tree_data() {
    let directory = tempdir().unwrap();
    let parent = directory.path().join("projects");
    fs::create_dir(&parent).unwrap();
    let registry_path = directory.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(&registry_path).unwrap();
    let project = service.create_project(&parent, "Branch Coherence").unwrap();
    let project_root = parent.join("Branch Coherence");
    let scene = service
        .create_scene(
            project.id,
            project_protocol::ProjectWriteTarget::Project(project.id),
            "Architecture",
        )
        .unwrap();

    run_git(&project_root, &["add", "."]);
    run_git(&project_root, &["commit", "-m", "Architecture A"]);
    run_git(&project_root, &["branch", "revised"]);
    run_git(&project_root, &["checkout", "revised"]);
    service
        .rename(
            project.id,
            project_protocol::ProjectWriteTarget::Scene(scene.scene_id),
            "Architecture Revised",
        )
        .unwrap();
    run_git(&project_root, &["add", "."]);
    run_git(&project_root, &["commit", "-m", "Architecture B"]);
    let revision_b = usd_git::Repository::open(&project_root)
        .unwrap()
        .head()
        .unwrap()
        .unwrap()
        .id()
        .to_string();
    run_git(&project_root, &["checkout", "main"]);

    let branch_b = service
        .switch_branch(project.id, "revised")
        .expect("branch B switches cleanly");
    assert_eq!(branch_b.project.id, project.id);
    assert_eq!(branch_b.project.name, "Branch Coherence");
    assert_eq!(
        branch_b.repository.active_branch.as_deref(),
        Some("revised")
    );
    assert_eq!(branch_b.repository.head.as_ref().unwrap().id, revision_b);
    assert!(branch_b.nodes.iter().any(|node| {
        matches!(
            node,
            usd_project::ProjectContentNode::Scene { name, .. }
                if name == "Architecture Revised"
        )
    }));
    assert_eq!(
        branch_b.project.counts, branch_b.counts,
        "summary and tree counts must describe the same branch snapshot"
    );

    let branch_a = service
        .switch_branch(project.id, "main")
        .expect("branch A switches cleanly");
    assert_eq!(branch_a.repository.active_branch.as_deref(), Some("main"));
    assert_ne!(
        branch_a.repository.head.as_ref().unwrap().id,
        branch_b.repository.head.as_ref().unwrap().id
    );
    assert!(branch_a.nodes.iter().any(|node| {
        matches!(
            node,
            usd_project::ProjectContentNode::Scene { name, .. }
                if name == "Architecture"
        )
    }));
    assert!(!branch_a.nodes.iter().any(|node| {
        matches!(
            node,
            usd_project::ProjectContentNode::Scene { name, .. }
                if name == "Architecture Revised"
        )
    }));
    assert_eq!(branch_a.project.counts, branch_a.counts);
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
