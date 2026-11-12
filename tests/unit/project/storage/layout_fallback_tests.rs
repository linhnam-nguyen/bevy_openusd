use std::fs;

use tempfile::tempdir;
use usd_project::{ProjectManifestV1, ProjectRoot, SceneId, SceneManifestEntry, StorageKey};

use super::*;

#[test]
fn canonical_manifest_blocks_legacy_asset_fallbacks() {
    let directory = tempdir().unwrap();
    let layout = ProjectStorageLayout::new(directory.path());
    let scene = SceneManifestEntry {
        id: SceneId::new_v4(),
        storage_key: StorageKey::new("Scene").unwrap(),
        display_name: "Scene".to_owned(),
    };
    let manifest = ProjectManifestV1::new(
        usd_project::ProjectId::new_v4(),
        "Project",
        ProjectRoot::Empty,
        vec![scene.clone()],
        vec![],
    );
    fs::write(layout.canonical_manifest_path(), b"canonical").unwrap();
    fs::create_dir_all(layout.legacy_scene_path(scene.id).parent().unwrap()).unwrap();
    fs::write(layout.legacy_scene_path(scene.id), b"legacy").unwrap();
    fs::create_dir_all(layout.legacy_scene_import_dir(scene.id)).unwrap();

    assert_eq!(
        layout.readable_manifest_path(),
        layout.canonical_manifest_path()
    );
    assert_eq!(
        layout.readable_scene_path(&manifest, &scene),
        layout.canonical_scene_path(&scene.storage_key)
    );
    assert_eq!(
        layout.readable_scene_import_dir(scene.id),
        layout.canonical_scene_import_dir(scene.id)
    );
}
