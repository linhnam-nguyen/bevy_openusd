use std::{collections::HashMap, fs};

use openusd::{sdf::Value, usd::Stage};
use tempfile::tempdir;
use usd_project::{
    ModelId, ModelManifestEntry, ModelSourceKind, ProjectId, ProjectManifestV1, ProjectRoot,
    SceneManifestEntry, StorageKey,
};

use super::*;

#[test]
fn migrated_legacy_scene_composes_content_and_is_idempotent() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let scene_id = SceneId::new_v4();
    let original = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Project",
        ProjectRoot::Scene(scene_id),
        vec![SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("Existing Scene")?,
        }],
        Vec::new(),
    );
    let legacy_path = authoring::scene_path(directory.path(), scene_id);
    fs::create_dir_all(legacy_path.parent().expect("Scene directory"))?;
    authoring::author_scene_atomic(directory.path(), scene_id)?;
    let legacy_stage = Stage::open(legacy_path.to_string_lossy().as_ref())?;
    legacy_stage
        .define_prim("/SceneRoot/LegacyBuilding/Wall")?
        .set_type_name("Xform")?;
    legacy_stage.set_default_prim("SceneRoot")?;
    legacy_stage
        .root_layer()
        .export(legacy_path.to_string_lossy().as_ref())?;

    let migrated = ensure_protected_root_scene_atomic(directory.path(), &original)?;
    let ProjectRoot::Scene(root_id) = migrated.root else {
        panic!("legacy Project must receive a protected Root Scene");
    };
    let root_path = authoring::scene_path(directory.path(), root_id);
    let members = authoring::read_scene_members(&root_path, root_id)?;
    let member = members
        .iter()
        .find(|member| member.target == SceneMemberTarget::Scene(scene_id))
        .expect("migrated Scene placement");
    assert_eq!(member.transform, Default::default());

    let root_stage = Stage::open(root_path.to_string_lossy().as_ref())?;
    let member_path = authoring::scene_member_path(member.id);
    let member_spec_path = openusd::sdf::path(member_path.as_str())?;
    assert!(
        root_stage
            .root_layer()
            .prim(&member_spec_path)
            .is_some_and(|spec| spec.has_field("references"))
    );
    assert!(
        root_stage
            .prim(member_path.as_str())
            .child_names()?
            .iter()
            .any(|name| name.as_str() == "LegacyBuilding")
    );

    let root_bytes = fs::read(&root_path)?;
    let repeated = ensure_protected_root_scene_atomic(directory.path(), &migrated)?;
    assert_eq!(repeated, migrated);
    assert_eq!(fs::read(root_path)?, root_bytes);
    assert_eq!(
        authoring::read_scene_members(&authoring::scene_path(directory.path(), root_id), root_id)?
            .len(),
        1
    );
    Ok(())
}

#[test]
fn migrated_legacy_model_composes_content_and_is_idempotent() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let model_id = ModelId::new_v4();
    let original = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Project",
        ProjectRoot::Model(model_id),
        Vec::new(),
        vec![ModelManifestEntry {
            id: model_id,
            source_kind: ModelSourceKind::Usd,
            storage_key: StorageKey::new("Existing Model")?,
        }],
    );
    let wrapper = model_wrapper_path(directory.path(), model_id);
    fs::create_dir_all(wrapper.parent().expect("Model wrapper directory"))?;
    let legacy_stage = Stage::builder().in_memory("legacy-model.usda")?;
    legacy_stage
        .define_prim("/ModelRoot")?
        .set_type_name("Xform")?
        .set_metadata(
            "customData",
            Value::Dictionary(HashMap::from([
                (
                    "usdhub:modelId".to_owned(),
                    Value::String(model_id.to_string()),
                ),
                ("usdhub:schemaVersion".to_owned(), Value::Int(1)),
            ])),
        )?;
    legacy_stage
        .define_prim("/ModelRoot/LegacyAsset/Mesh")?
        .set_type_name("Xform")?;
    legacy_stage.set_default_prim("ModelRoot")?;
    legacy_stage
        .root_layer()
        .export(wrapper.to_string_lossy().as_ref())?;

    let migrated = ensure_protected_root_scene_atomic(directory.path(), &original)?;
    let ProjectRoot::Scene(root_id) = migrated.root else {
        panic!("legacy Project must receive a protected Root Scene");
    };
    assert!(migrated.models.iter().any(|entry| entry.id == model_id));
    let root_path = authoring::scene_path(directory.path(), root_id);
    let members = authoring::read_scene_members(&root_path, root_id)?;
    let member = members
        .iter()
        .find(|member| member.target == SceneMemberTarget::Model(model_id))
        .expect("migrated Model placement");
    assert_eq!(member.transform, Default::default());

    let root_stage = Stage::open(root_path.to_string_lossy().as_ref())?;
    let member_path = authoring::scene_member_path(member.id);
    let member_spec_path = openusd::sdf::path(member_path.as_str())?;
    assert!(
        root_stage
            .root_layer()
            .prim(&member_spec_path)
            .is_some_and(|spec| spec.has_field("references"))
    );
    assert!(
        root_stage
            .prim(member_path.as_str())
            .child_names()?
            .iter()
            .any(|name| name.as_str() == "LegacyAsset")
    );

    let root_bytes = fs::read(&root_path)?;
    let repeated = ensure_protected_root_scene_atomic(directory.path(), &migrated)?;
    assert_eq!(repeated, migrated);
    assert_eq!(fs::read(root_path)?, root_bytes);
    assert_eq!(
        authoring::read_scene_members(&authoring::scene_path(directory.path(), root_id), root_id)?
            .len(),
        1
    );
    Ok(())
}
