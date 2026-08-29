use std::fs;

use anyhow::Result;
use tempfile::tempdir;
use usd_project::{
    ModelManifestEntry, ModelSourceKind, ProjectId, ProjectManifestV1, ProjectRoot, SceneId,
    StorageKey,
};

use super::*;
use crate::project::catalog::manifest_store::ManifestStore;

#[test]
fn imported_scene_dependency_closure_changes_only_composed_targets() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_a = SceneId::new_v4();
    let scene_b = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Imported Scene Closure",
        ProjectRoot::Scene(scene_a),
        vec![
            usd_project::SceneManifestEntry {
                id: scene_a,
                storage_key: StorageKey::new("scene-a")?,
                display_name: "Scene A".to_owned(),
            },
            usd_project::SceneManifestEntry {
                id: scene_b,
                storage_key: StorageKey::new("scene-b")?,
                display_name: "Scene B".to_owned(),
            },
        ],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    let wrapper_a =
        crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_a)?;
    crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_b)?;

    let imports = directory
        .path()
        .join(".usdhub/imports/scenes")
        .join(scene_a.to_string());
    fs::create_dir_all(&imports)?;
    let source_a = imports.join("source.usda");
    fs::write(&source_a, b"imported-source-v1")?;
    let imports_b = directory
        .path()
        .join(".usdhub/imports/scenes")
        .join(scene_b.to_string());
    fs::create_dir_all(&imports_b)?;
    let source_b = imports_b.join("source.usda");
    fs::write(&source_b, b"sibling-source-v1")?;

    let target_a = ProjectCacheTarget::Scene {
        id: scene_a.to_string(),
    };
    let target_b = ProjectCacheTarget::Scene {
        id: scene_b.to_string(),
    };
    let root = ProjectCacheTarget::ProjectRoot;
    let identity_a_v1 = ProjectCacheIdentity::for_project(
        directory.path(),
        target_a.clone(),
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    let root_v1 = ProjectCacheIdentity::for_project(
        directory.path(),
        root.clone(),
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    let identity_b_v1 = ProjectCacheIdentity::for_project(
        directory.path(),
        target_b.clone(),
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    let wrapper_bytes = fs::read(&wrapper_a)?;

    fs::write(&source_a, b"imported-source-v2")?;
    let identity_a_v2 = ProjectCacheIdentity::for_project(
        directory.path(),
        target_a.clone(),
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    let root_v2 = ProjectCacheIdentity::for_project(
        directory.path(),
        root.clone(),
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    assert_ne!(identity_a_v1, identity_a_v2);
    assert_ne!(root_v1, root_v2);
    assert_eq!(fs::read(&wrapper_a)?, wrapper_bytes);

    fs::write(&source_a, b"imported-source-v1")?;
    let identity_a_restored = ProjectCacheIdentity::for_project(
        directory.path(),
        target_a.clone(),
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    assert_eq!(identity_a_v1, identity_a_restored);

    fs::write(&source_b, b"sibling-source-v2")?;
    let identity_a_after_sibling_edit = ProjectCacheIdentity::for_project(
        directory.path(),
        target_a,
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    assert_eq!(identity_a_restored, identity_a_after_sibling_edit);
    assert_eq!(identity_b_v1.target, target_b);
    Ok(())
}

#[test]
fn target_cache_identity_changes_when_authoritative_display_name_changes() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = SceneId::new_v4();
    let project_id = ProjectId::new_v4();
    let manifest = ProjectManifestV1::new(
        project_id,
        "Identity Project",
        ProjectRoot::Scene(scene_id),
        vec![usd_project::SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("architecture")?,
            display_name: "Architecture".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_id)?;

    let target = ProjectCacheTarget::Scene {
        id: scene_id.to_string(),
    };
    let before = ProjectCacheIdentity::for_project(
        directory.path(),
        target.clone(),
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;

    let mut renamed = manifest;
    renamed.scenes[0].display_name = "Architecture Revised".to_owned();
    ManifestStore::write_manifest_atomic(directory.path(), &renamed)?;
    let after = ProjectCacheIdentity::for_project(
        directory.path(),
        target,
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;

    assert_ne!(
        before.target_content_hash, after.target_content_hash,
        "stale runtime labels must not survive a canonical name change"
    );
    Ok(())
}

#[test]
fn activation_preparation_returns_fallback_when_runtime_warm_fails() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let model_id = usd_project::ModelId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Failed Runtime Warm",
        ProjectRoot::Model(model_id),
        Vec::new(),
        vec![ModelManifestEntry {
            id: model_id,
            source_kind: ModelSourceKind::Usd,
            storage_key: StorageKey::new("model")?,
            display_name: "Model".to_owned(),
        }],
    );
    ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
    let wrapper = crate::project::model_wrapper::model_wrapper_path(directory.path(), model_id);
    fs::create_dir_all(wrapper.parent().expect("Model wrapper directory"))?;
    fs::write(&wrapper, b"#usda 1.0\n(this is not a valid USD layer")?;

    let target = ProjectCacheTarget::Model {
        id: model_id.to_string(),
    };
    let queue = ProjectCacheWarmQueue::default();
    assert_eq!(
        queue.prepare_for_activation(directory.path(), target.clone()),
        ProjectCachePreparation::FallbackRequired
    );
    let identity = ProjectCacheIdentity::for_project(
        directory.path(),
        target,
        RuntimeProfile::NativeMedium,
        SemanticConfig::default().hash(),
    )?;
    let descriptor = ProjectCacheStore::new(directory.path())
        .load(&identity)?
        .expect("failed warm publishes a descriptor");
    assert_eq!(descriptor.state, ProjectCacheState::FallbackRequired);
    Ok(())
}
