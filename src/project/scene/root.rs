//! Project composition-root authoring and legacy-root migration.
//!
//! A protected Root Scene is an implementation detail of the Project
//! aggregate. It owns the top-level placements while preserving every
//! existing Scene and Model identity during migration.

use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use openusd::{sdf::Value, usd::Stage};
use usd_project::{
    ProjectManifestV1, ProjectRoot, SceneCompositionGraph, SceneId, SceneManifestEntry,
    SceneMember, SceneMemberId, SceneMemberTarget, StorageKey,
};

use super::authoring;
use crate::project::{
    catalog::manifest_store::ManifestStore, model_wrapper::model_wrapper_path,
    storage::ProjectStorageLayout,
};

/// Create the protected Root Scene when a Project is first published, or
/// migrate a legacy Empty/Scene/Model root without changing existing IDs.
pub(crate) fn ensure_protected_root_scene_atomic(
    project_root: &Path,
    base_manifest: &ProjectManifestV1,
) -> Result<ProjectManifestV1> {
    let migrated_base = base_manifest
        .clone()
        .migrate_legacy()
        .context("migrate base Project manifest before Root Scene migration")?;
    let base_manifest = &migrated_base;
    base_manifest
        .validate()
        .context("validate base Project manifest before Root Scene migration")?;

    if let ProjectRoot::Scene(scene_id) = base_manifest.root
        && is_protected_root_scene(
            &protected_root_path(project_root, base_manifest, scene_id),
            scene_id,
        )?
    {
        return Ok(base_manifest.clone());
    }

    let root_scene_id = SceneId::new_v4();
    let members = legacy_root_members(project_root, base_manifest)?;
    let mut next_manifest = base_manifest.clone();
    next_manifest.scenes.push(SceneManifestEntry {
        id: root_scene_id,
        display_name: base_manifest.name.clone(),
        storage_key: protected_root_storage_key(base_manifest, root_scene_id)?,
    });
    next_manifest.root = ProjectRoot::Scene(root_scene_id);
    next_manifest
        .validate()
        .context("validate migrated Project Root Scene manifest")?;

    let root_entry = next_manifest
        .scenes
        .iter()
        .find(|scene| scene.id == root_scene_id)
        .expect("new protected Root Scene entry exists");
    let root_path = authoring::scene_path_for_entry(project_root, root_entry, true);
    ensure!(
        !root_path.exists(),
        "generated protected Root Scene path already exists"
    );
    authoring::author_scene_atomic_at_path(
        project_root,
        &root_path,
        root_scene_id,
        &SceneCompositionGraph::default(),
        &members,
        true,
        Some(&base_manifest.name),
    )?;

    if let Err(error) = ManifestStore::write_manifest_atomic(project_root, &next_manifest) {
        let _ = fs::remove_file(&root_path);
        return Err(error).context("publish Project manifest after Root Scene migration");
    }

    Ok(next_manifest)
}

fn protected_root_path(
    project_root: &Path,
    manifest: &ProjectManifestV1,
    scene_id: SceneId,
) -> std::path::PathBuf {
    let layout = ProjectStorageLayout::new(project_root);
    let Some(scene) = manifest.scenes.iter().find(|scene| scene.id == scene_id) else {
        return layout.legacy_scene_path(scene_id);
    };
    let canonical = layout.canonical_root_scene_path(&scene.storage_key);
    if canonical.exists() || !layout.legacy_scene_path(scene_id).exists() {
        canonical
    } else {
        layout.legacy_scene_path(scene_id)
    }
}

pub(crate) fn is_protected_root_scene(path: &Path, expected_scene_id: SceneId) -> Result<bool> {
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string).context("open Project root Scene")?;
    let root = stage.prim("/SceneRoot");
    let Some(Value::Dictionary(custom_data)) = root.custom_data()? else {
        anyhow::bail!("Project root Scene is missing customData");
    };
    let Some(scene_id) = custom_data.get("usdhub:sceneId").and_then(Value::as_str) else {
        anyhow::bail!("Project root Scene is missing usdhub:sceneId");
    };
    ensure!(
        SceneId::parse(scene_id)? == expected_scene_id,
        "Project root Scene metadata identity does not match the manifest"
    );
    Ok(custom_data.get("usdhub:protectedRoot") == Some(&Value::Bool(true)))
}

fn legacy_root_members(
    project_root: &Path,
    manifest: &ProjectManifestV1,
) -> Result<Vec<SceneMember>> {
    let member = match manifest.root {
        ProjectRoot::Empty => None,
        ProjectRoot::Scene(scene_id) => {
            let scene = manifest
                .scenes
                .iter()
                .find(|entry| entry.id == scene_id)
                .with_context(|| format!("legacy root Scene {scene_id} is not registered"))?;
            ensure!(
                ProjectStorageLayout::new(project_root)
                    .readable_scene_path(manifest, scene)
                    .is_file(),
                "legacy root Scene layer is missing"
            );
            Some(SceneMember {
                id: SceneMemberId::new_v4(),
                target: SceneMemberTarget::Scene(scene_id),
                name: Some(scene.display_name.clone()),
                transform: Default::default(),
            })
        }
        ProjectRoot::Model(model_id) => {
            let model = manifest
                .models
                .iter()
                .find(|entry| entry.id == model_id)
                .with_context(|| format!("legacy root Model {model_id} is not registered"))?;
            ensure!(
                model_wrapper_path(project_root, model_id).is_file(),
                "legacy root Model wrapper is missing"
            );
            Some(SceneMember {
                id: SceneMemberId::new_v4(),
                target: SceneMemberTarget::Model(model_id),
                name: Some(model.display_name.clone()),
                transform: Default::default(),
            })
        }
    };
    Ok(member.into_iter().collect())
}

fn protected_root_storage_key(
    manifest: &ProjectManifestV1,
    root_scene_id: SceneId,
) -> Result<StorageKey> {
    if !manifest
        .scenes
        .iter()
        .any(|entry| entry.storage_key.as_str() == manifest.name)
        && !manifest
            .models
            .iter()
            .any(|entry| entry.storage_key.as_str() == manifest.name)
    {
        return StorageKey::new(manifest.name.clone()).context("create protected Root Scene name");
    }
    StorageKey::new(format!("__usdhub_root_{root_scene_id}"))
        .context("create unique protected Root Scene name")
}

#[cfg(test)]
mod tests {
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
        let members = authoring::read_scene_members(
            &authoring::scene_path(directory.path(), root_id),
            root_id,
        )
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
        let members = authoring::read_scene_members(
            &authoring::scene_path(directory.path(), root_id),
            root_id,
        )
        .unwrap();
        assert!(members.iter().any(|member| {
            member.target == SceneMemberTarget::Model(model_id)
                && member.name.as_deref() == Some("Existing Model")
        }));
    }
}

#[cfg(test)]
#[path = "root_c7_tests.rs"]
mod c7_tests;
