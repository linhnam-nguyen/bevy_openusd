use super::*;
use crate::{ModelId, ProjectId};

fn storage_key(value: &str) -> StorageKey {
    StorageKey::new(value).unwrap()
}

#[test]
fn canonical_json_is_independent_of_insertion_order() {
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
    let model_a = ModelManifestEntry {
        id: ModelId::new_v4(),
        source_kind: ModelSourceKind::Usd,
        storage_key: storage_key("model-a"),
        display_name: "Model A".to_owned(),
    };
    let left = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Project",
        ProjectRoot::Empty,
        vec![scene_a.clone(), scene_b.clone()],
        vec![model_a.clone()],
    );
    let right = ProjectManifestV1::new(
        left.project_id,
        left.name.clone(),
        ProjectRoot::Empty,
        vec![scene_b, scene_a],
        vec![model_a],
    );

    let left_json = serde_json::to_string_pretty(&left.canonicalized()).unwrap();
    let right_json = serde_json::to_string_pretty(&right.canonicalized()).unwrap();

    assert_eq!(left_json, right_json);
}

#[test]
fn unsafe_storage_keys_are_rejected() {
    for value in [
        "",
        ".",
        "..",
        "/absolute",
        "\\absolute",
        "nested/key",
        "C:drive",
        "nul\0",
    ] {
        assert!(
            StorageKey::new(value).is_err(),
            "accepted unsafe key {value:?}"
        );
    }
}

#[test]
fn schema_version_must_be_exactly_current() {
    let mut manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Project",
        ProjectRoot::Empty,
        Vec::new(),
        Vec::new(),
    );
    manifest.schema_version = 3;

    assert!(matches!(
        manifest.validate_schema_version(),
        Err(ProjectManifestError::UnsupportedSchemaVersion { actual: 3, .. })
    ));
}

#[test]
fn legacy_manifest_migration_derives_names_without_changing_identity() {
    let scene_id = SceneId::new_v4();
    let model_id = ModelId::new_v4();
    let mut legacy = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Project",
        ProjectRoot::Scene(scene_id),
        vec![SceneManifestEntry {
            id: scene_id,
            storage_key: storage_key("legacy-scene"),
            display_name: String::new(),
        }],
        vec![ModelManifestEntry {
            id: model_id,
            source_kind: crate::ModelSourceKind::Usd,
            storage_key: storage_key("legacy-model"),
            display_name: String::new(),
        }],
    );
    legacy.schema_version = 1;

    let migrated = legacy.migrate_legacy().unwrap();

    assert_eq!(migrated.schema_version, PROJECT_MANIFEST_SCHEMA_VERSION);
    assert_eq!(migrated.scenes[0].id, scene_id);
    assert_eq!(migrated.scenes[0].display_name, "legacy-scene");
    assert_eq!(migrated.models[0].id, model_id);
    assert_eq!(migrated.models[0].display_name, "legacy-model");
}
