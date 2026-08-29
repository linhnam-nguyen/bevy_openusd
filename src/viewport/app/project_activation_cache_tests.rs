use std::fs;

use project_protocol::{ProjectActivationCommand, ProjectStageTarget};
use tempfile::tempdir;
use usd_project::{
    ProjectId, ProjectManifestV1, ProjectRoot, SceneId, SceneManifestEntry, StorageKey,
};
use viewport_protocol::SessionId;

use super::*;
use crate::project::cache::{
    ProjectCacheDescriptor, ProjectCacheIdentity, ProjectCacheState, ProjectCacheStore,
    ProjectCacheTarget,
};
use crate::project::cache_hydration::default_project_cache_config_hash;
use crate::project::cache_warmer::ProjectCacheWarmQueue;
use crate::project::catalog::{
    manifest_store::ManifestStore, workspace_registry::WorkspaceRegistry,
};
use crate::project::scene::authoring::author_scene_atomic;

#[test]
fn immediate_activation_waits_for_an_inflight_cache_warm() {
    let directory = tempdir().expect("temporary activation workspace");
    let registry_path = directory.path().join("workspace.json");
    let project_root = directory.path().join("project");
    usd_git::Repository::init(&project_root).expect("initialize Project repository");
    let project_id = ProjectId::new_v4();
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        project_id,
        "Immediate Cache Activation",
        ProjectRoot::Scene(scene_id),
        vec![SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("scene").expect("Scene storage key"),
            display_name: "Immediate Cache Activation".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(&project_root, &manifest).expect("write Project manifest");
    let scene_path = author_scene_atomic(&project_root, scene_id).expect("author Scene");
    let target = ProjectCacheTarget::Scene {
        id: scene_id.to_string(),
    };
    let identity = ProjectCacheIdentity::for_project(
        &project_root,
        target.clone(),
        viewport_protocol::RuntimeProfile::NativeMedium,
        default_project_cache_config_hash(),
    )
    .expect("compute cache identity");
    ProjectCacheStore::new(&project_root)
        .publish(
            &ProjectCacheDescriptor::new(identity.clone(), ProjectCacheState::Building, None)
                .expect("create Building descriptor"),
        )
        .expect("publish Building descriptor");

    let warm_queue = ProjectCacheWarmQueue::default();
    assert!(warm_queue.enqueue(&project_root, target));
    let mut registry = WorkspaceRegistry::load(&registry_path).expect("load registry");
    registry
        .register(project_id, &project_root, None)
        .expect("register Project");
    let request = ProjectActivationRequest {
        session_id: SessionId::new("session-immediate-cache"),
        command: ProjectActivationCommand::new(
            "activation-immediate-cache",
            1,
            project_id,
            ProjectStageTarget::Scene(scene_id),
        ),
    };

    let resolved = resolve_project_activation(Some(&registry_path), &request)
        .expect("activation preparation succeeds")
        .expect("Scene activation target");
    let descriptor = ProjectCacheStore::new(&project_root)
        .load(&identity)
        .expect("load prepared descriptor")
        .expect("prepared descriptor");
    assert_eq!(descriptor.state, ProjectCacheState::Ready);
    assert_eq!(
        resolved.path,
        fs::canonicalize(scene_path).expect("canonical Scene path")
    );
}
