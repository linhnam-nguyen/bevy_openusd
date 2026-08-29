//! USD authoring and validation helpers for stable Model wrappers.

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use openusd::{
    sdf,
    sdf::Value,
    usd::{InitialLoadSet, Stage},
};

use super::{
    MODEL_ID_METADATA, MODEL_ROOT_PRIM, MODEL_SCHEMA_VERSION, REFERENCES_FIELD,
    SCHEMA_VERSION_METADATA, SOURCE_PRIM,
};

pub(super) fn source_default_prim(source: &Path) -> Result<String> {
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

pub(super) fn copy_file_synced(source: &Path, destination: &Path) -> Result<()> {
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

pub(super) fn author_model_wrapper(
    path: &Path,
    model_id: usd_project::ModelId,
    source_path: &str,
    source_default_prim: &str,
    spatial: &usd_project::SourceSpatialConvention,
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
    crate::project::spatial::author_canonical_stage(&stage)?;
    let source_prim = stage
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
    crate::project::spatial::author_source_normalization(&source_prim, spatial)?;
    stage
        .root_layer()
        .export(path.to_string_lossy().as_ref())
        .context("export temporary stable Model wrapper")?;
    Ok(())
}

pub(super) fn validate_model_wrapper(
    path: &Path,
    model_id: usd_project::ModelId,
    spatial: &usd_project::SourceSpatialConvention,
) -> Result<()> {
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
            .prim(&source_path)
            .is_some_and(|spec| spec.has_field(REFERENCES_FIELD)),
        "stable Model wrapper must reference its source"
    );
    let source_prim = stage.prim(source_path.as_str());
    ensure!(
        crate::project::spatial::read_source_normalization(&source_prim)?
            == crate::project::spatial::source_normalization_transform(spatial),
        "stable Model wrapper spatial normalization does not match the inspected source"
    );
    Ok(())
}
