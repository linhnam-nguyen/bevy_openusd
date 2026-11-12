use std::{collections::HashSet, fs};

use tempfile::tempdir;
use usd_project::{
    ModelManifestEntry, ModelSourceKind, ProjectId, ProjectManifestV1, ProjectRoot, SceneId,
    SceneManifestEntry, StorageKey,
};

use super::*;

fn storage_key(value: &str) -> StorageKey {
    StorageKey::new(value).unwrap()
}

fn manifest() -> ProjectManifestV1 {
    let scene_a = SceneManifestEntry {
        id: SceneId::new_v4(),
        storage_key: storage_key("scene-a"),
        display_name: "Scene A".to_owned(),
    };
    let scene_b = SceneManifestEntry {
        id: SceneId::new_v4(),
        storage_key: storage_key("scene-b"),
        display_name: "Scene B".to_owned(),
    };
    ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Project",
        ProjectRoot::Empty,
        vec![scene_b, scene_a],
        vec![ModelManifestEntry {
            id: usd_project::ModelId::new_v4(),
            source_kind: ModelSourceKind::Usd,
            storage_key: storage_key("model"),
            display_name: "Model".to_owned(),
        }],
    )
}

#[test]
fn writes_canonical_manifest_to_same_metadata_directory() {
    let directory = tempdir().unwrap();
    let manifest = manifest();

    ManifestStore::write_manifest_atomic(directory.path(), &manifest).unwrap();

    let path = super::manifest_path(directory.path());
    let bytes = fs::read(&path).unwrap();
    let decoded: ProjectManifestV1 = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, manifest.canonicalized());
    assert!(
        fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().starts_with(".project."))
    );
}

#[test]
fn invalid_manifest_does_not_replace_existing_file() {
    let directory = tempdir().unwrap();
    let valid = manifest();
    ManifestStore::write_manifest_atomic(directory.path(), &valid).unwrap();
    let path = super::manifest_path(directory.path());
    let before = fs::read(&path).unwrap();

    let mut invalid = valid;
    invalid.schema_version = 3;
    assert!(ManifestStore::write_manifest_atomic(directory.path(), &invalid).is_err());

    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(
        fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().starts_with(".project."))
    );
}

#[test]
fn reading_a_manifest_persists_deterministic_display_names() {
    let directory = tempdir().unwrap();
    let manifest = manifest();
    let path = super::manifest_path(directory.path());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut legacy = serde_json::to_value(&manifest).unwrap();
    legacy["schema_version"] = serde_json::json!(1);
    for collection in ["scenes", "models"] {
        for entry in legacy[collection].as_array_mut().unwrap() {
            entry.as_object_mut().unwrap().remove("display_name");
        }
    }
    fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();

    let validated = ManifestStore::read_validated(directory.path()).unwrap();

    assert_eq!(
        validated.raw().schema_version,
        usd_project::PROJECT_MANIFEST_SCHEMA_VERSION
    );
    let scene_names = validated
        .raw()
        .scenes
        .iter()
        .map(|scene| scene.display_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        scene_names.into_iter().collect::<HashSet<_>>(),
        HashSet::from(["scene-a", "scene-b"])
    );
    assert_eq!(validated.raw().models[0].display_name, "model");
    let persisted: ProjectManifestV1 = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(persisted, validated.raw().clone());
}
