//! Stable USDHub Model wrappers.
//!
//! A wrapper owns the Project Model identity and contains one opaque source
//! reference. It does not turn composed source dependencies into additional
//! Project Models or flatten the source Stage.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use usd_project::{
    ModelManifestEntry, ProjectManifestV1, ProjectRoot, SceneId, SceneMember, SceneMemberTarget,
    ScenePlacementTransform, StorageKey,
};
use uuid::Uuid;

use super::model_import::{ModelImporter, UsdModelImporter};
use crate::project::{
    catalog::manifest_store::ManifestStore,
    scene::{adoption_authoring, authoring},
    source_closure::materialize_source_closure,
    storage::ProjectStorageLayout,
};

#[path = "model_wrapper_authoring.rs"]
mod wrapper_authoring;

const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";
const TRANSACTIONS_DIRECTORY: &str = ".transactions";
const MODELS_DIRECTORY: &str = "models";
const MODEL_ROOT_PRIM: &str = "ModelRoot";
const SOURCE_PRIM: &str = "Source";
const MODEL_ID_METADATA: &str = "usdhub:modelId";
const SCHEMA_VERSION_METADATA: &str = "usdhub:schemaVersion";
const MODEL_SCHEMA_VERSION: i32 = 1;
const REFERENCES_FIELD: &str = "references";

pub(crate) fn model_wrapper_path(project_root: &Path, model_id: usd_project::ModelId) -> PathBuf {
    let layout = ProjectStorageLayout::new(project_root);
    if let Ok(manifest) = ManifestStore::read_validated(project_root)
        && let Some(model) = manifest.model(model_id)
    {
        let canonical = layout.canonical_model_wrapper_path(model);
        return canonical;
    }
    if layout.canonical_manifest_present() {
        // Do not turn a missing or invalid canonical wrapper into a legacy
        // read.
        return layout
            .canonical_models_dir()
            .join(model_id.to_string())
            .join("model.usda");
    }
    layout.legacy_model_wrapper_path(model_id)
}

/// Inputs for publishing one stable Model wrapper.
pub(crate) struct ModelWrapperRequest<'a> {
    pub project_root: &'a Path,
    pub base_manifest: &'a ProjectManifestV1,
    pub prepared: &'a super::model_import::PreparedModel,
    pub set_as_root: bool,
    pub placement: Option<ModelPlacement<'a>>,
}

/// Existing authored Scene layer that receives a new Model placement.
pub(crate) struct ModelPlacement<'a> {
    pub parent_scene_id: SceneId,
    pub parent_members: &'a [SceneMember],
    pub transform: ScenePlacementTransform,
}

/// Published Model identity and canonical wrapper location.
#[derive(Clone, Debug)]
pub(crate) struct PublishedModel {
    pub id: usd_project::ModelId,
    pub wrapper_path: PathBuf,
    pub placement: Option<SceneMember>,
    pub manifest: ProjectManifestV1,
}

/// Publish a stable wrapper and register its single Model identity.
pub(crate) fn publish_model_wrapper_atomic(
    request: ModelWrapperRequest<'_>,
) -> Result<PublishedModel> {
    request
        .base_manifest
        .validate()
        .context("validate base Project manifest")?;
    ensure_current_manifest(request.project_root, request.base_manifest)?;

    let importer = UsdModelImporter;
    ensure!(
        request.prepared.source_kind == importer.kind(),
        "prepared Model uses an importer kind not supported by the USD wrapper"
    );
    ensure!(
        request.prepared.source.is_file(),
        "prepared Model source disappeared or is not a file"
    );
    let model_name = request.prepared.name.trim();
    ensure!(
        !model_name.is_empty(),
        "published Model name must not be empty"
    );
    let storage_key = next_model_storage_key(request.base_manifest, model_name)?;
    let model_name = storage_key.as_str().to_owned();
    let revalidated = importer.inspect(&request.prepared.source)?;
    ensure!(
        revalidated == request.prepared.inspection,
        "prepared Model source changed after importer preparation"
    );
    ensure!(
        !request
            .base_manifest
            .models
            .iter()
            .any(|entry| entry.id == request.prepared.id),
        "prepared Model identity is already registered in the Project manifest"
    );
    if let Some(placement) = request.placement.as_ref() {
        ensure!(
            request
                .base_manifest
                .scenes
                .iter()
                .any(|entry| entry.id == placement.parent_scene_id),
            "Model placement parent Scene is not registered in the Project manifest"
        );
        ensure!(
            !request.set_as_root,
            "a Model placement cannot also replace the Project root"
        );
    }

    let source_default_prim = wrapper_authoring::source_default_prim(&request.prepared.source)?;
    let model_directory = ProjectStorageLayout::new(request.project_root)
        .canonical_models_dir()
        .join(model_name.as_str());
    ensure!(
        !model_directory.exists(),
        "canonical Model wrapper directory already exists"
    );

    let placement = request
        .placement
        .as_ref()
        .map(|placement_request| SceneMember {
            id: usd_project::SceneMemberId::new_v4(),
            target: SceneMemberTarget::Model(request.prepared.id),
            name: Some(model_name.clone()),
            transform: placement_request.transform,
        });
    let parent_members = request.placement.as_ref().map(|placement_request| {
        let mut members = placement_request.parent_members.to_vec();
        members.push(
            placement
                .as_ref()
                .expect("Model placement request creates a placement")
                .clone(),
        );
        members
    });

    let mut manifest_candidate = request.base_manifest.clone();
    manifest_candidate.models.push(ModelManifestEntry {
        id: request.prepared.id,
        source_kind: request.prepared.source_kind.clone(),
        storage_key,
        display_name: model_name.clone(),
    });
    if request.set_as_root {
        manifest_candidate.root = ProjectRoot::Model(request.prepared.id);
    }
    manifest_candidate
        .validate()
        .context("validate proposed Model manifest")?;

    let transaction_directory = request
        .project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(TRANSACTIONS_DIRECTORY)
        .join(Uuid::new_v4().to_string());
    let temporary_model_directory = transaction_directory
        .join("models")
        .join(request.prepared.id.to_string());
    fs::create_dir_all(&temporary_model_directory)
        .context("create Model wrapper transaction directory")?;
    let temporary_source_directory = transaction_directory
        .join("imports")
        .join(MODELS_DIRECTORY)
        .join(request.prepared.id.to_string());
    let temporary_source_path = temporary_source_directory.join("model.usda");
    let temporary_wrapper_path = temporary_model_directory.join("model.usda");
    let source_directory = ProjectStorageLayout::new(request.project_root)
        .canonical_model_import_dir(request.prepared.id);
    let parent_scene_path = request
        .placement
        .as_ref()
        .map(|placement| authoring::scene_path(request.project_root, placement.parent_scene_id));
    let temporary_parent_path = request.placement.as_ref().map(|placement| {
        transaction_directory
            .join("scenes")
            .join(format!("{}.usda", placement.parent_scene_id))
    });
    let parent_backup_path = transaction_directory.join("parent-backup.usda");
    let mut parent_published = false;
    let mut parent_backup_created = false;
    let mut model_published = false;
    let mut source_published = false;

    if let Some(path) = temporary_parent_path.as_ref() {
        fs::create_dir_all(path.parent().expect("temporary parent path has a parent"))
            .context("create Model placement transaction directory")?;
    }

    let result = (|| {
        let mut source_name =
            materialize_source_closure(&request.prepared.source, &temporary_source_directory)?;
        if request
            .prepared
            .inspection
            .composition
            .dependencies
            .is_empty()
            && source_name != "model.usda"
        {
            fs::rename(
                temporary_source_directory.join(&source_name),
                &temporary_source_path,
            )
            .context("normalize controlled Model source filename")?;
            source_name = "model.usda".to_owned();
        }
        wrapper_authoring::author_model_wrapper(
            &temporary_wrapper_path,
            request.project_root,
            &model_directory.join("model.usda"),
            request.prepared.id,
            &source_directory.join(&source_name),
            &source_default_prim,
            &model_name,
            &request.prepared.inspection.composition.spatial,
        )?;
        wrapper_authoring::validate_model_wrapper(
            &temporary_wrapper_path,
            request.prepared.id,
            &request.prepared.inspection.composition.spatial,
        )?;

        if let (Some(parent_path), Some(temporary_parent_path), Some(parent_members)) = (
            parent_scene_path.as_ref(),
            temporary_parent_path.as_ref(),
            parent_members.as_deref(),
        ) {
            adoption_authoring::prepare_parent_layer_with_model_path(
                parent_path,
                temporary_parent_path,
                request.project_root,
                request
                    .placement
                    .as_ref()
                    .expect("parent path implies Model placement")
                    .parent_scene_id,
                parent_members,
                request.prepared.id,
                &model_directory.join("model.usda"),
            )?;
            authoring::validate_scene_file(
                temporary_parent_path,
                request
                    .placement
                    .as_ref()
                    .expect("parent path implies Model placement")
                    .parent_scene_id,
                parent_members,
            )?;
        }

        if let Some(parent_path) = parent_scene_path.as_ref() {
            ensure!(
                parent_path.exists(),
                "Model placement parent Scene layer is missing"
            );
            fs::copy(parent_path, &parent_backup_path)
                .context("backup parent Scene layer before Model publication")?;
            parent_backup_created = true;
            fs::rename(
                temporary_parent_path
                    .as_ref()
                    .expect("parent path implies temporary parent path"),
                parent_path,
            )
            .context("publish updated parent Scene layer")?;
            parent_published = true;
        }

        let models_directory = model_directory
            .parent()
            .context("canonical Model directory has no parent")?;
        let imports_directory = source_directory
            .parent()
            .context("canonical Model import directory has no parent")?;
        fs::create_dir_all(imports_directory)
            .context("create canonical Model imports directory")?;
        fs::rename(&temporary_source_directory, &source_directory)
            .context("publish canonical Model import closure")?;
        source_published = true;
        fs::create_dir_all(models_directory).context("create canonical Model directory")?;
        fs::rename(&temporary_model_directory, &model_directory)
            .context("publish canonical Model wrapper directory")?;
        model_published = true;
        if manifest_candidate != request.base_manifest.canonicalized() {
            ManifestStore::write_manifest_atomic(request.project_root, &manifest_candidate)
                .context("publish Model Project manifest")?;
        }
        Ok(())
    })();

    let final_result = match result {
        Ok(()) => Ok(PublishedModel {
            id: request.prepared.id,
            wrapper_path: model_directory.join("model.usda"),
            placement,
            manifest: manifest_candidate,
        }),
        Err(error) => {
            if model_published && model_directory.exists() {
                let _ = fs::remove_dir_all(&model_directory);
            }
            if source_published && source_directory.exists() {
                let _ = fs::remove_dir_all(&source_directory);
            }
            if parent_published {
                if parent_backup_created {
                    let _ = fs::remove_file(
                        &parent_scene_path
                            .as_ref()
                            .expect("published parent path exists"),
                    );
                    let _ = fs::rename(
                        &parent_backup_path,
                        parent_scene_path
                            .as_ref()
                            .expect("published parent path exists"),
                    );
                } else if let Some(parent_path) = parent_scene_path.as_ref() {
                    let _ = fs::remove_file(parent_path);
                }
            }
            Err(error)
        }
    };
    let _ = fs::remove_dir_all(&transaction_directory);
    final_result
}

fn next_model_storage_key(manifest: &ProjectManifestV1, name: &str) -> Result<StorageKey> {
    for ordinal in 1_u32.. {
        let candidate = if ordinal == 1 {
            name.to_owned()
        } else {
            format!("{name} ({ordinal})")
        };
        let storage_key = StorageKey::new(candidate)?;
        let already_used = manifest
            .scenes
            .iter()
            .any(|entry| entry.storage_key == storage_key)
            || manifest
                .models
                .iter()
                .any(|entry| entry.storage_key == storage_key);
        if !already_used {
            return Ok(storage_key);
        }
    }
    unreachable!("u32 storage-key suffix space exhausted")
}

fn ensure_current_manifest(project_root: &Path, expected: &ProjectManifestV1) -> Result<()> {
    let current = ManifestStore::read_validated(project_root)
        .context("read current Project manifest before Model publication")?;
    ensure!(
        current.raw() == &expected.canonicalized(),
        "Project manifest changed after Model preparation"
    );
    Ok(())
}

#[cfg(test)]
#[path = "model_wrapper_tests.rs"]
mod tests;
