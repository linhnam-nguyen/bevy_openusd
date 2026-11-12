use std::fs;

use tempfile::tempdir;
use usd_project::{
    ModelId, ModelManifestEntry, ModelSourceKind, ProjectId, ProjectManifestV1, ProjectRoot,
    SceneManifestEntry, StorageKey,
};

use super::*;

fn manifest(root: ProjectRoot) -> ProjectManifestV1 {
    ProjectManifestV1::new(ProjectId::new_v4(), "Project", root, Vec::new(), Vec::new())
}

#[test]
fn creates_and_marks_a_protected_root_scene() {
    let directory = tempdir().unwrap();
    let original = manifest(ProjectRoot::Empty);

    let migrated = ensure_protected_root_scene_atomic(directory.path(), &original).unwrap();
    let ProjectRoot::Scene(root_id) = migrated.root else {
        panic!("Project must publish a protected Root Scene");
    };

    assert_eq!(migrated.scenes.len(), 1);
    assert!(
        is_protected_root_scene(&authoring::scene_path(directory.path(), root_id), root_id)
            .unwrap()
    );
    assert_eq!(
        ManifestStore::read_validated(directory.path())
            .unwrap()
            .raw(),
        &migrated
    );
    assert_eq!(migrated.scenes[0].storage_key.as_str(), "Project");
    assert_eq!(
        ensure_protected_root_scene_atomic(directory.path(), &migrated).unwrap(),
        migrated
    );
}

#[test]
fn migrates_legacy_scene_root_without_reinterpreting_its_identity() {
    let directory = tempdir().unwrap();
    let scene_id = SceneId::new_v4();
    let original = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Project",
        ProjectRoot::Scene(scene_id),
        vec![SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("Existing Scene").unwrap(),
            display_name: "Existing Scene".to_owned(),
        }],
        Vec::new(),
    );
    authoring::author_scene_atomic(directory.path(), scene_id).unwrap();

    let migrated = ensure_protected_root_scene_atomic(directory.path(), &original).unwrap();
    let ProjectRoot::Scene(root_id) = migrated.root else {
        panic!("legacy Project must receive a protected Root Scene");
    };
    assert_ne!(root_id, scene_id);
    assert!(migrated.scenes.iter().any(|entry| entry.id == scene_id));
    let members =
        authoring::read_scene_members(&authoring::scene_path(directory.path(), root_id), root_id)
            .unwrap();
    assert!(members.iter().any(|member| {
        member.target == SceneMemberTarget::Scene(scene_id)
            && member.name.as_deref() == Some("Existing Scene")
    }));
}

#[test]
fn migrates_legacy_model_root_without_changing_model_identity() {
    let directory = tempdir().unwrap();
    let model_id = ModelId::new_v4();
    let original = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Project",
        ProjectRoot::Model(model_id),
        Vec::new(),
        vec![ModelManifestEntry {
            id: model_id,
            source_kind: ModelSourceKind::Usd,
            storage_key: StorageKey::new("Existing Model").unwrap(),
            display_name: "Existing Model".to_owned(),
        }],
    );
    let wrapper = model_wrapper_path(directory.path(), model_id);
    fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
    fs::write(&wrapper, b"legacy model wrapper").unwrap();

    let migrated = ensure_protected_root_scene_atomic(directory.path(), &original).unwrap();
    let ProjectRoot::Scene(root_id) = migrated.root else {
        panic!("legacy Project must receive a protected Root Scene");
    };
    assert!(migrated.models.iter().any(|entry| entry.id == model_id));
    let members =
        authoring::read_scene_members(&authoring::scene_path(directory.path(), root_id), root_id)
            .unwrap();
    assert!(members.iter().any(|member| {
        member.target == SceneMemberTarget::Model(model_id)
            && member.name.as_deref() == Some("Existing Model")
    }));
}
