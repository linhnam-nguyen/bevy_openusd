//! Stable USDHub Model wrappers.
//!
//! A wrapper owns the Project Model identity and contains one opaque source
//! reference. It does not turn composed source dependencies into additional
//! Project Models or flatten the source Stage.

use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use openusd::{
    sdf,
    sdf::Value,
    usd::{InitialLoadSet, Stage},
};
use usd_project::{ModelManifestEntry, ProjectManifestV1, ProjectRoot, StorageKey};
use uuid::Uuid;

use super::model_import::{ModelImporter, UsdModelImporter};
use crate::project::catalog::manifest_store::ManifestStore;

const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";
const TRANSACTIONS_DIRECTORY: &str = ".transactions";
const MODELS_DIRECTORY: &str = "models";
const MODEL_ROOT_PRIM: &str = "ModelRoot";
const SOURCE_PRIM: &str = "Source";
const MODEL_ID_METADATA: &str = "usdhub:modelId";
const SCHEMA_VERSION_METADATA: &str = "usdhub:schemaVersion";
const MODEL_SCHEMA_VERSION: i32 = 1;
const REFERENCES_FIELD: &str = "references";

/// Inputs for publishing one stable Model wrapper.
pub(crate) struct ModelWrapperRequest<'a> {
    pub project_root: &'a Path,
    pub base_manifest: &'a ProjectManifestV1,
    pub prepared: &'a super::model_import::PreparedModel,
    pub set_as_root: bool,
}

/// Published Model identity and canonical wrapper location.
#[derive(Clone, Debug)]
pub(crate) struct PublishedModel {
    pub id: usd_project::ModelId,
    pub wrapper_path: PathBuf,
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

    let source_default_prim = source_default_prim(&request.prepared.source)?;
    let model_directory = request
        .project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(MODELS_DIRECTORY)
        .join(request.prepared.id.to_string());
    ensure!(
        !model_directory.exists(),
        "canonical Model wrapper directory already exists"
    );

    let mut manifest_candidate = request.base_manifest.clone();
    manifest_candidate.models.push(ModelManifestEntry {
        id: request.prepared.id,
        source_kind: request.prepared.source_kind.clone(),
        storage_key: StorageKey::new(request.prepared.id.to_string())?,
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
        .join(MODELS_DIRECTORY)
        .join(request.prepared.id.to_string());
    fs::create_dir_all(&temporary_model_directory)
        .context("create Model wrapper transaction directory")?;
    let temporary_source_directory = temporary_model_directory.join("source");
    let temporary_source_path = temporary_source_directory.join("model.usda");
    let temporary_wrapper_path = temporary_model_directory.join("model.usda");

    let result = (|| {
        let copy_source = request
            .prepared
            .inspection
            .composition
            .dependencies
            .is_empty();
        let published_source_path = if copy_source {
            fs::create_dir_all(&temporary_source_directory)
                .context("create controlled Model source directory")?;
            copy_file_synced(&request.prepared.source, &temporary_source_path)?;
            "./source/model.usda".to_owned()
        } else {
            request
                .prepared
                .source
                .to_str()
                .context("Model source path must be valid UTF-8")?
                .to_owned()
        };

        author_model_wrapper(
            &temporary_wrapper_path,
            request.prepared.id,
            &published_source_path,
            &source_default_prim,
        )?;
        validate_model_wrapper(&temporary_wrapper_path, request.prepared.id)?;
        let models_directory = model_directory
            .parent()
            .context("canonical Model directory has no parent")?;
        fs::create_dir_all(models_directory).context("create canonical Model directory")?;
        fs::rename(&temporary_model_directory, &model_directory)
            .context("publish canonical Model wrapper directory")?;
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
            manifest: manifest_candidate,
        }),
        Err(error) => {
            if model_directory.exists() {
                let _ = fs::remove_dir_all(&model_directory);
            }
            Err(error)
        }
    };
    let _ = fs::remove_dir_all(&transaction_directory);
    final_result
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

fn source_default_prim(source: &Path) -> Result<String> {
    let source_string = source
        .to_str()
        .context("Model source path must be valid UTF-8")?;
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(source_string)
        .context("open prepared Model source")?;
    stage
        .default_prim()
        .map(|token| token.as_str().to_owned())
        .context("prepared Model source has no defaultPrim")
}

fn copy_file_synced(source: &Path, destination: &Path) -> Result<()> {
    let mut input = File::open(source)
        .with_context(|| format!("open controlled Model source {}", source.display()))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .with_context(|| format!("create controlled Model source {}", destination.display()))?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).context("read Model source")?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .context("copy Model source")?;
    }
    output.sync_all().context("sync controlled Model source")?;
    Ok(())
}

fn author_model_wrapper(
    path: &Path,
    model_id: usd_project::ModelId,
    source_path: &str,
    source_default_prim: &str,
) -> Result<()> {
    let stage = Stage::builder().in_memory(format!("model-{model_id}.usda"))?;
    stage
        .define_prim(format!("/{MODEL_ROOT_PRIM}").as_str())?
        .set_type_name("Xform")?
        .set_metadata(
            "customData",
            Value::Dictionary(HashMap::from([
                (
                    MODEL_ID_METADATA.to_owned(),
                    Value::String(model_id.to_string()),
                ),
                (
                    SCHEMA_VERSION_METADATA.to_owned(),
                    Value::Int(MODEL_SCHEMA_VERSION),
                ),
            ])),
        )?;
    stage.set_default_prim(MODEL_ROOT_PRIM)?;
    stage
        .define_prim(format!("/{MODEL_ROOT_PRIM}/{SOURCE_PRIM}").as_str())?
        .set_type_name("Xform")?
        .set_metadata(
            REFERENCES_FIELD,
            Value::ReferenceListOp(sdf::ReferenceListOp::prepended([sdf::Reference {
                asset_path: source_path.to_owned(),
                prim_path: sdf::path(format!("/{source_default_prim}"))?,
                ..Default::default()
            }])),
        )?;
    stage
        .root_layer()
        .export(path.to_string_lossy().as_ref())
        .context("export temporary stable Model wrapper")?;
    Ok(())
}

fn validate_model_wrapper(path: &Path, model_id: usd_project::ModelId) -> Result<()> {
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(&path_string)
        .context("reopen stable Model wrapper")?;
    ensure!(
        stage
            .default_prim()
            .as_ref()
            .is_some_and(|token| token.as_str() == MODEL_ROOT_PRIM),
        "stable Model wrapper defaultPrim must be /{MODEL_ROOT_PRIM}"
    );
    let root = stage.prim(format!("/{MODEL_ROOT_PRIM}").as_str());
    ensure!(
        root.is_defined()?,
        "stable Model wrapper root must be defined"
    );
    let Some(Value::Dictionary(data)) = root.custom_data()? else {
        anyhow::bail!("stable Model wrapper root is missing customData");
    };
    ensure!(
        data.get(MODEL_ID_METADATA) == Some(&Value::String(model_id.to_string())),
        "stable Model wrapper identity does not match its directory"
    );
    ensure!(
        data.get(SCHEMA_VERSION_METADATA) == Some(&Value::Int(MODEL_SCHEMA_VERSION)),
        "stable Model wrapper schema version is unsupported or missing"
    );
    let source_path = sdf::path(format!("/{MODEL_ROOT_PRIM}/{SOURCE_PRIM}"))?;
    ensure!(
        stage
            .root_layer()
            .prim(source_path)
            .is_some_and(|spec| spec.has_field(REFERENCES_FIELD)),
        "stable Model wrapper must reference its source"
    );
    Ok(())
}

#[cfg(test)]
#[path = "model_wrapper_tests.rs"]
mod tests;
