use std::fs;

use tempfile::tempdir;

use super::*;

#[test]
fn storage_v2_paths_are_keyed_by_stable_storage_identity() {
    let layout = ProjectStorageLayout::new("project");
    let scene = SceneManifestEntry {
        id: SceneId::new_v4(),
        storage_key: StorageKey::new("Lv1").unwrap(),
        display_name: "Level One".to_owned(),
    };
    let model = ModelManifestEntry {
        id: ModelId::new_v4(),
        source_kind: usd_project::ModelSourceKind::Usd,
        storage_key: StorageKey::new("Chair").unwrap(),
        display_name: "Chair".to_owned(),
    };

    assert_eq!(
        layout.canonical_manifest_path(),
        PathBuf::from("project/project.json")
    );
    assert_eq!(
        layout.canonical_root_scene_path(&StorageKey::new("Pro2").unwrap()),
        PathBuf::from("project/Pro2.usda")
    );
    assert_eq!(
        layout.scene_path(&scene),
        PathBuf::from("project/scenes/Lv1.usda")
    );
    assert_eq!(
        layout.canonical_model_wrapper_path(&model),
        PathBuf::from("project/models/Chair/model.usda")
    );
    assert_eq!(
        layout.canonical_scene_import_dir(scene.id),
        PathBuf::from(format!("project/imports/scenes/{}", scene.id))
    );
    assert_eq!(
        layout.legacy_scene_path(scene.id),
        PathBuf::from(format!("project/.usdhub/scenes/{}.usda", scene.id))
    );
}

#[test]
fn authored_project_asset_paths_are_relative_and_contained() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    let authoring = root.join("scenes/Lv1.usda");
    let target = root.join("models/Chair/model.usda");

    assert_eq!(
        authored_relative_project_asset_path(root, &authoring, &target).unwrap(),
        "../models/Chair/model.usda"
    );
    assert!(
        authored_relative_project_asset_path(root, &authoring, Path::new("/tmp/a.usda")).is_err()
    );
}

#[test]
fn local_state_roots_are_bootstrapped_without_object_or_session_children() {
    let directory = tempdir().unwrap();
    let layout = ProjectStorageLayout::new(directory.path());
    layout.ensure_local_state_roots().unwrap();

    assert!(layout.cache_dir().is_dir());
    assert!(layout.recovery_dir().is_dir());
    assert!(!layout.cache_objects_dir().exists());
    assert_eq!(fs::read_dir(layout.recovery_dir()).unwrap().count(), 0);
}

#[test]
fn legacy_manifest_accessor_remains_separate_from_storage_v2() {
    let layout = ProjectStorageLayout::new("project");
    assert_eq!(
        layout.manifest_path(),
        PathBuf::from("project/project.json")
    );
    assert_eq!(
        layout.legacy_manifest_path(),
        PathBuf::from("project/.usdhub/project.json")
    );
}
