use anyhow::Result;
use tempfile::tempdir;
use usd_project::{ProjectId, ProjectManifestV1, ProjectRoot, SceneId};
use viewport_protocol::RuntimeProfile;

use super::*;

#[test]
fn unrelated_scene_edits_keep_a_sibling_cache_identity_reusable() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Warm Project",
        ProjectRoot::Empty,
        vec![
            usd_project::SceneManifestEntry {
                id: SceneId::new_v4(),
                storage_key: usd_project::StorageKey::new("first").unwrap(),
                display_name: "First".to_owned(),
            },
            usd_project::SceneManifestEntry {
                id: SceneId::new_v4(),
                storage_key: usd_project::StorageKey::new("second").unwrap(),
                display_name: "Second".to_owned(),
            },
        ],
        Vec::new(),
    );
    let first_scene = manifest.scenes[0].id;
    let second_scene = manifest.scenes[1].id;
    crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
        directory.path(),
        &manifest,
    )?;
    let first_path =
        crate::project::scene::authoring::author_scene_atomic(directory.path(), first_scene)?;
    let second_path =
        crate::project::scene::authoring::author_scene_atomic(directory.path(), second_scene)?;
    let first_identity = ProjectCacheIdentity::for_project(
        directory.path(),
        ProjectCacheTarget::Scene {
            id: first_scene.to_string(),
        },
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    let store = ProjectCacheStore::new(directory.path());
    store.publish(&ProjectCacheDescriptor::new(
        first_identity.clone(),
        ProjectCacheState::Partial,
        None,
    )?)?;

    std::fs::write(second_path, b"unrelated sibling edit")?;
    let unchanged_identity = ProjectCacheIdentity::for_project(
        directory.path(),
        ProjectCacheTarget::Scene {
            id: first_scene.to_string(),
        },
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    assert_eq!(first_identity, unchanged_identity);
    assert!(store.load(&unchanged_identity)?.is_some());
    assert!(first_path.is_file());
    Ok(())
}
