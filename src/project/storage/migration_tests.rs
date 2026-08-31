use std::{collections::HashMap, fs, path::Path, sync::Mutex};

use anyhow::Result;
use openusd::{sdf, sdf::Value, usd::Stage};
use tempfile::{TempDir, tempdir};
use usd_project::{
    ModelId, ModelManifestEntry, ModelSourceKind, ProjectId, ProjectManifestV1, ProjectRoot,
    SceneId, SceneManifestEntry, SceneMember, SceneMemberTarget, StorageKey,
};

use super::*;
use crate::project::{catalog::manifest_store::ManifestStore, scene::authoring};

static MIGRATION_TEST_LOCK: Mutex<()> = Mutex::new(());

struct LegacyFixture {
    directory: TempDir,
    manifest: ProjectManifestV1,
    root_scene_id: SceneId,
    child_scene_id: SceneId,
    model_id: ModelId,
    scene_member_id: usd_project::SceneMemberId,
    model_member_id: usd_project::SceneMemberId,
}

fn legacy_fixture() -> LegacyFixture {
    let directory = tempdir().unwrap();
    let root_scene_id = SceneId::new_v4();
    let child_scene_id = SceneId::new_v4();
    let model_id = ModelId::new_v4();
    let scene_member_id = usd_project::SceneMemberId::new_v4();
    let model_member_id = usd_project::SceneMemberId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Pro2",
        ProjectRoot::Scene(root_scene_id),
        vec![
            SceneManifestEntry {
                id: root_scene_id,
                storage_key: StorageKey::new("Pro2").unwrap(),
                display_name: "Pro2".to_owned(),
            },
            SceneManifestEntry {
                id: child_scene_id,
                storage_key: StorageKey::new("Lv1").unwrap(),
                display_name: "Lv1".to_owned(),
            },
        ],
        vec![ModelManifestEntry {
            id: model_id,
            source_kind: ModelSourceKind::Usd,
            storage_key: StorageKey::new("Chair").unwrap(),
            display_name: "Chair".to_owned(),
        }],
    );
    let layout = ProjectStorageLayout::new(directory.path());
    fs::create_dir_all(layout.scenes_dir()).unwrap();
    fs::create_dir_all(layout.legacy_model_wrapper_path(model_id).parent().unwrap()).unwrap();
    fs::create_dir_all(layout.legacy_scene_import_dir(root_scene_id)).unwrap();
    fs::create_dir_all(layout.legacy_model_import_dir(model_id)).unwrap();
    fs::create_dir_all(layout.metadata_dir().join("links")).unwrap();
    fs::write(
        layout.legacy_manifest_path(),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    write_empty_scene(&layout.legacy_scene_path(child_scene_id), child_scene_id);
    write_legacy_scene(
        &layout.legacy_scene_path(root_scene_id),
        root_scene_id,
        &[
            (
                SceneMember {
                    id: scene_member_id,
                    target: SceneMemberTarget::Scene(child_scene_id),
                    name: Some("Lv1".to_owned()),
                    transform: Default::default(),
                },
                format!("{}.usda", child_scene_id),
                "/SceneRoot",
            ),
            (
                SceneMember {
                    id: model_member_id,
                    target: SceneMemberTarget::Model(model_id),
                    name: Some("Chair".to_owned()),
                    transform: Default::default(),
                },
                format!("../models/{model_id}/model.usda"),
                "/ModelRoot",
            ),
        ],
    );
    let scene_source = layout
        .legacy_scene_import_dir(root_scene_id)
        .join("Projet1.usdc");
    write_empty_scene(&scene_source, SceneId::new_v4());
    let model_source = layout.legacy_model_import_dir(model_id).join("model.usda");
    write_empty_model(&model_source, model_id);
    write_legacy_model(
        &layout.legacy_model_wrapper_path(model_id),
        model_id,
        "../../imports/models/PLACEHOLDER/model.usda",
    );
    let wrapper = layout.legacy_model_wrapper_path(model_id);
    rewrite_placeholder_reference(&wrapper, model_id);

    LegacyFixture {
        directory,
        manifest,
        root_scene_id,
        child_scene_id,
        model_id,
        scene_member_id,
        model_member_id,
    }
}

fn write_empty_scene(path: &Path, scene_id: SceneId) {
    let stage = authoring::new_scene_stage(scene_id).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    stage
        .root_layer()
        .export(path.to_string_lossy().as_ref())
        .unwrap();
}

fn write_empty_model(path: &Path, model_id: ModelId) {
    let stage = Stage::builder().in_memory("source-model.usda").unwrap();
    stage.define_prim("/ModelRoot").unwrap();
    stage.set_default_prim("ModelRoot").unwrap();
    let _ = model_id;
    stage
        .root_layer()
        .export(path.to_string_lossy().as_ref())
        .unwrap();
}

fn write_legacy_scene(path: &Path, scene_id: SceneId, members: &[(SceneMember, String, &str)]) {
    let stage = authoring::new_scene_stage(scene_id).unwrap();
    let root = stage.prim("/SceneRoot");
    let mut data = match root.custom_data().unwrap() {
        Some(Value::Dictionary(data)) => data,
        _ => HashMap::new(),
    };
    data.insert("usdhub:schemaVersion".to_owned(), Value::Int(1));
    root.set_metadata("customData", Value::Dictionary(data))
        .unwrap();
    for (member, asset_path, prim_path) in members {
        let member_path = format!(
            "/SceneRoot/Members/Member_{}",
            member.id.to_string().replace('-', "")
        );
        stage
            .define_prim(member_path.as_str())
            .unwrap()
            .set_type_name("Xform")
            .unwrap()
            .set_metadata(
                "customData",
                Value::Dictionary(authoring::member_custom_data(member)),
            )
            .unwrap()
            .set_metadata(
                "references",
                Value::ReferenceListOp(sdf::ReferenceListOp::prepended([sdf::Reference {
                    asset_path: asset_path.clone(),
                    prim_path: sdf::path(*prim_path).unwrap(),
                    ..Default::default()
                }])),
            )
            .unwrap();
    }
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    stage
        .root_layer()
        .export(path.to_string_lossy().as_ref())
        .unwrap();
}

fn write_legacy_model(path: &Path, model_id: ModelId, asset_path: &str) {
    let stage = Stage::builder().in_memory("legacy-model.usda").unwrap();
    stage
        .define_prim("/ModelRoot")
        .unwrap()
        .set_type_name("Xform")
        .unwrap()
        .set_metadata(
            "customData",
            Value::Dictionary(HashMap::from([(
                "usdhub:modelId".to_owned(),
                Value::String(model_id.to_string()),
            )])),
        )
        .unwrap();
    stage
        .define_prim("/ModelRoot/Source")
        .unwrap()
        .set_type_name("Xform")
        .unwrap()
        .set_metadata(
            "references",
            Value::ReferenceListOp(sdf::ReferenceListOp::prepended([sdf::Reference {
                asset_path: asset_path.to_owned(),
                prim_path: sdf::path("/ModelRoot").unwrap(),
                ..Default::default()
            }])),
        )
        .unwrap();
    stage.set_default_prim("ModelRoot").unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    stage
        .root_layer()
        .export(path.to_string_lossy().as_ref())
        .unwrap();
}

fn rewrite_placeholder_reference(path: &Path, model_id: ModelId) {
    let stage = Stage::open(path.to_string_lossy().as_ref()).unwrap();
    let mut refs = {
        let root_layer = stage.root_layer();
        let spec = root_layer
            .prim(&sdf::path("/ModelRoot/Source").unwrap())
            .unwrap();
        let Some(Value::ReferenceListOp(refs)) = spec.field("references").unwrap() else {
            panic!("legacy Model Source reference missing");
        };
        refs
    };
    let reference = refs
        .iter_mut()
        .next()
        .expect("legacy Model Source reference");
    reference.asset_path = format!("../../imports/models/{model_id}/model.usda");
    stage
        .prim("/ModelRoot/Source")
        .set_metadata("references", Value::ReferenceListOp(refs))
        .unwrap();
    stage
        .root_layer()
        .export(path.to_string_lossy().as_ref())
        .unwrap();
}

#[test]
fn migrates_legacy_project_to_storage_v2_and_is_idempotent() -> Result<()> {
    let _guard = MIGRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = legacy_fixture();
    let root = fixture.directory.path();
    let migrated = ManifestStore::read_validated(root)?;
    assert_eq!(migrated.raw(), &fixture.manifest.canonicalized());
    let layout = ProjectStorageLayout::new(root);
    assert!(layout.canonical_manifest_path().is_file());
    assert!(!layout.legacy_manifest_path().exists());
    assert!(!layout.legacy_scene_path(fixture.root_scene_id).exists());
    assert!(
        !layout
            .legacy_scene_import_dir(fixture.root_scene_id)
            .exists()
    );
    assert!(!layout.legacy_model_import_dir(fixture.model_id).exists());
    assert!(
        layout
            .canonical_root_scene_path(&StorageKey::new("Pro2")?)
            .is_file()
    );
    assert!(
        layout
            .canonical_scene_path(&StorageKey::new("Lv1")?)
            .is_file()
    );
    assert!(layout.canonical_model_import_dir(fixture.model_id).is_dir());

    let scene_path = layout.canonical_root_scene_path(&StorageKey::new("Pro2")?);
    let stage = Stage::open(scene_path.to_string_lossy().as_ref())?;
    assert!(!stage.prim("/SceneRoot/Members").is_defined()?);
    assert!(
        stage
            .prim(authoring::scene_member_path(fixture.scene_member_id).as_str())
            .is_defined()?,
        "migrated root layer:\n{}",
        fs::read_to_string(&scene_path)?
    );
    assert!(
        stage
            .prim(authoring::scene_member_path(fixture.model_member_id).as_str())
            .is_defined()?,
        "migrated root layer:\n{}",
        fs::read_to_string(&scene_path)?
    );
    let members = authoring::read_scene_members(&scene_path, fixture.root_scene_id)?;
    assert_eq!(members.len(), 2);
    let before = fs::read(layout.canonical_manifest_path())?;
    let _ = ManifestStore::read_validated(root)?;
    assert_eq!(fs::read(layout.canonical_manifest_path())?, before);
    Ok(())
}

#[test]
fn migration_failure_after_ordinary_scene_restores_legacy_tree() {
    let _guard = MIGRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = legacy_fixture();
    let root = fixture.directory.path();
    super::failure_injection::set(1);
    assert!(ManifestStore::read_validated(root).is_err());
    let layout = ProjectStorageLayout::new(root);
    assert!(layout.legacy_manifest_path().is_file());
    assert!(!layout.canonical_manifest_path().exists());
    assert!(layout.legacy_scene_path(fixture.child_scene_id).is_file());
    assert!(!layout.canonical_scenes_dir().join("Lv1.usda").exists());
}

#[test]
fn migration_failure_before_manifest_restores_legacy_tree() {
    let _guard = MIGRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = legacy_fixture();
    let root = fixture.directory.path();
    super::failure_injection::set(2);
    assert!(ManifestStore::read_validated(root).is_err());
    let layout = ProjectStorageLayout::new(root);
    assert!(layout.legacy_manifest_path().is_file());
    assert!(!layout.canonical_manifest_path().exists());
    assert!(
        layout
            .legacy_scene_import_dir(fixture.root_scene_id)
            .is_dir()
    );
    assert!(layout.legacy_model_wrapper_path(fixture.model_id).is_file());
}

#[test]
fn interrupted_migration_is_recovered_before_restart() -> Result<()> {
    let _guard = MIGRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = legacy_fixture();
    let root = fixture.directory.path();
    let layout = ProjectStorageLayout::new(root);
    let migrated_manifest = fixture.manifest.clone().migrate_legacy()?.canonicalized();
    let transaction_directory = layout
        .metadata_dir()
        .join(".transactions")
        .join("migration-interrupted");
    let plan = super::build_plan(root, &migrated_manifest, transaction_directory.clone())?;
    super::publish::write_journal(&plan)?;
    super::failure_injection::set(1);
    assert!(super::publish::publish_plan(root, &migrated_manifest, &plan).is_err());
    assert!(!layout.canonical_manifest_path().exists());
    assert!(transaction_directory.is_dir());

    let migrated = ManifestStore::read_validated(root)?;

    assert_eq!(migrated.raw(), &migrated_manifest);
    assert!(layout.canonical_manifest_path().is_file());
    assert!(!layout.legacy_manifest_path().exists());
    assert!(!transaction_directory.exists());
    assert!(
        layout
            .canonical_root_scene_path(&StorageKey::new("Pro2")?)
            .is_file()
    );
    Ok(())
}

#[test]
fn committed_manifest_wins_over_a_stale_legacy_manifest_after_restart() -> Result<()> {
    let _guard = MIGRATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = legacy_fixture();
    let root = fixture.directory.path();
    let layout = ProjectStorageLayout::new(root);
    let migrated_manifest = fixture.manifest.clone().migrate_legacy()?.canonicalized();
    let transaction_directory = layout
        .metadata_dir()
        .join(".transactions")
        .join("migration-committed");
    let plan = super::build_plan(root, &migrated_manifest, transaction_directory.clone())?;
    super::publish::write_journal(&plan)?;
    super::failure_injection::set(3);
    assert!(super::publish::publish_plan(root, &migrated_manifest, &plan).is_err());
    assert!(layout.canonical_manifest_path().is_file());
    assert!(layout.legacy_manifest_path().is_file());

    let read = ManifestStore::read_validated(root)?;

    assert_eq!(read.raw(), &migrated_manifest);
    assert!(!transaction_directory.exists());
    Ok(())
}

#[path = "migration_recovery_tests.rs"]
mod recovery_tests;
