//! USD authoring and validation helpers for stable Model wrappers.

use std::{collections::HashMap, path::Path};

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

pub(super) fn source_entrypoint_prims(source: &Path) -> Result<Vec<String>> {
    let source_string = source
        .to_str()
        .context("Model source path must be valid UTF-8")?;
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(source_string)
        .context("open prepared Model source")?;
    crate::project::scene::adoption_support::source_entrypoint_prims(&stage)
}

pub(super) fn author_model_wrapper(
    path: &Path,
    project_root: &Path,
    authored_layer_path: &Path,
    model_id: usd_project::ModelId,
    source_path: &Path,
    source_metadata_path: &Path,
    source_prims: &[String],
    model_name: &str,
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
    stage
        .prim(format!("/{MODEL_ROOT_PRIM}").as_str())
        .set_metadata("ui:displayName", Value::String(model_name.to_owned()))?;
    stage.set_default_prim(MODEL_ROOT_PRIM)?;
    crate::project::spatial::author_canonical_stage(&stage)?;
    crate::project::stage_metadata::copy_source_time_metadata(source_metadata_path, &stage)?;
    let source_path = crate::project::storage::authored_relative_project_asset_path(
        project_root,
        authored_layer_path,
        source_path,
    )?;
    let source_prim = stage
        .define_prim(format!("/{MODEL_ROOT_PRIM}/{SOURCE_PRIM}").as_str())?
        .set_type_name("Xform")?;
    author_source_references(&stage, &source_prim, &source_path, source_prims)?;
    crate::project::spatial::author_source_normalization(&source_prim, spatial)?;
    crate::project::spatial::author_source_hierarchy_role(&source_prim)?;
    stage
        .root_layer()
        .export(path.to_string_lossy().as_ref())
        .context("export temporary stable Model wrapper")?;
    Ok(())
}

fn author_source_references(
    stage: &Stage,
    source_prim: &openusd::usd::Prim,
    source_path: &str,
    source_prims: &[String],
) -> Result<()> {
    ensure!(
        !source_prims.is_empty(),
        "Model source has no entrypoint prims"
    );
    if source_prims.len() == 1 {
        source_prim.clone().set_metadata(
            REFERENCES_FIELD,
            Value::ReferenceListOp(sdf::ReferenceListOp::prepended([sdf::Reference {
                asset_path: source_path.to_owned(),
                prim_path: sdf::path(&source_prims[0])?,
                ..Default::default()
            }])),
        )?;
        return Ok(());
    }

    for (index, source_path_name) in source_prims.iter().enumerate() {
        stage
            .define_prim(format!("/{MODEL_ROOT_PRIM}/{SOURCE_PRIM}/Root_{index}").as_str())?
            .set_type_name("Xform")?
            .set_metadata(
                REFERENCES_FIELD,
                Value::ReferenceListOp(sdf::ReferenceListOp::prepended([sdf::Reference {
                    asset_path: source_path.to_owned(),
                    prim_path: sdf::path(source_path_name)?,
                    ..Default::default()
                }])),
            )?;
    }
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
            .prim(source_path.as_str())
            .custom_data()?
            .and_then(|value| match value {
                Value::Dictionary(data) => data
                    .get(crate::project::spatial::USDHUB_HIERARCHY_ROLE_METADATA)
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                _ => None,
            })
            .as_deref()
            == Some(crate::project::spatial::USDHUB_TRANSPARENT_SOURCE_ROLE),
        "stable Model wrapper source must carry the transparent hierarchy role"
    );
    validate_source_reference_paths(&stage, &source_path)?;
    let source_prim = stage.prim(source_path.as_str());
    ensure!(
        crate::project::spatial::read_source_normalization(&source_prim)?
            == crate::project::spatial::source_normalization_transform(spatial),
        "stable Model wrapper spatial normalization does not match the inspected source"
    );
    Ok(())
}

fn validate_source_reference_paths(stage: &Stage, source_path: &sdf::Path) -> Result<()> {
    let root_layer = stage.root_layer();
    let mut specs = Vec::new();
    if let Some(spec) = root_layer.prim(source_path)
        && spec.has_field(REFERENCES_FIELD)
    {
        specs.push(spec);
    } else {
        for child in stage.prim(source_path.as_str()).children()? {
            if let Ok(path) = sdf::path(child.path().as_str())
                && let Some(spec) = root_layer.prim(&path)
                && spec.has_field(REFERENCES_FIELD)
            {
                specs.push(spec);
            }
        }
    }
    ensure!(
        !specs.is_empty(),
        "stable Model wrapper has no source references"
    );
    for spec in specs {
        let Some(Value::ReferenceListOp(references)) = spec.field(REFERENCES_FIELD)? else {
            anyhow::bail!("stable Model wrapper source reference list is missing");
        };
        for reference in references.iter() {
            ensure!(
                !reference.asset_path.is_empty() && !Path::new(&reference.asset_path).is_absolute(),
                "stable Model wrapper source asset path must be relative"
            );
        }
    }
    Ok(())
}
