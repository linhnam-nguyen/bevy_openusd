//! Canonical USDHub spatial metadata and source-wrapper normalization.

use std::path::Path;
use std::{collections::HashMap, f64::consts::FRAC_PI_2};

use anyhow::{Context, Result, bail, ensure};
use openusd::{
    gf::Matrix4d,
    sdf::Value,
    usd::{Prim, Stage},
};
use usd_project::{SourceSpatialConvention, StageUpAxis};

pub(crate) const USDHUB_UP_AXIS: StageUpAxis = StageUpAxis::Y;
pub(crate) const USDHUB_METERS_PER_UNIT: f64 = 1.0;
pub(crate) const USDHUB_HIERARCHY_ROLE_METADATA: &str = usd_bevy::USDHUB_HIERARCHY_ROLE_METADATA;
pub(crate) const USDHUB_TRANSPARENT_SOURCE_ROLE: &str = usd_bevy::USDHUB_TRANSPARENT_SOURCE_ROLE;
pub(crate) const USDHUB_SOURCE_BINDING_METADATA: &str = "usdhub:sourceBinding";
pub(crate) const USDHUB_LINKED_SOURCE_BINDING: &str = "linked";
const OPENUSD_UNAUTHORED_METERS_PER_UNIT: f64 = 0.01;

/// Read composed Stage metrics without carrying a Stage handle across a
/// protocol boundary. The fallback values match OpenUSD's metric semantics.
pub(crate) fn inspect_stage(stage: &Stage) -> Result<SourceSpatialConvention> {
    let mut up_axis_was_authored = false;
    let mut meters_per_unit_was_authored = false;
    for identifier in stage.layer_stack() {
        if let Some(layer) = stage.layer(&identifier)
            && let Some(root) = layer.pseudo_root()
        {
            up_axis_was_authored |= root.has_field("upAxis");
            meters_per_unit_was_authored |= root.has_field("metersPerUnit");
        }
    }

    let up_axis = match stage.stage_metadata("upAxis")? {
        Some(value) => parse_up_axis(value)?,
        None => USDHUB_UP_AXIS,
    };
    let meters_per_unit = match stage.stage_metadata("metersPerUnit")? {
        Some(value) => value
            .try_as_double()
            .context("USD metersPerUnit metadata must be a double")?,
        None => OPENUSD_UNAUTHORED_METERS_PER_UNIT,
    };
    ensure!(
        meters_per_unit.is_finite() && meters_per_unit > 0.0,
        "USD metersPerUnit must be finite and positive"
    );

    Ok(SourceSpatialConvention {
        up_axis,
        meters_per_unit,
        up_axis_was_authored,
        meters_per_unit_was_authored,
    })
}

pub(crate) fn inspect_source(path: &Path) -> Result<SourceSpatialConvention> {
    let source = path
        .to_str()
        .context("USD spatial inspection path must be valid UTF-8")?;
    let stage = Stage::open(source).context("open USD source for spatial inspection")?;
    inspect_stage(&stage)
}

/// Author the canonical metrics on a newly-created USDHub Stage.
pub(crate) fn author_canonical_stage(stage: &Stage) -> Result<()> {
    let root_identifier = stage.root_layer().identifier().to_owned();
    let mut root_layer = stage
        .layer_mut(&root_identifier)
        .context("USDHub Scene root layer is unavailable")?;
    root_layer
        .edit(|edit| {
            let mut root = edit.pseudo_root_mut()?;
            root.set("upAxis", Value::Token(canonical_up_axis_token().into()));
            root.set("metersPerUnit", Value::Double(USDHUB_METERS_PER_UNIT));
            Ok(())
        })
        .context("author canonical USDHub Stage metrics")?;
    Ok(())
}

/// Return the single stable corrective matrix used by Scene and Model wrappers.
/// OpenUSD's row-vector matrix convention makes -90 degrees around X map a
/// right-handed source Z-up basis to canonical Y-up: `(x, y, z) -> (x, z, -y)`.
pub(crate) fn source_normalization_transform(source: &SourceSpatialConvention) -> Matrix4d {
    let unit_scale = source.meters_per_unit / USDHUB_METERS_PER_UNIT;
    let scale = Matrix4d::scale([unit_scale, unit_scale, unit_scale]);
    match source.up_axis {
        StageUpAxis::Y => scale,
        StageUpAxis::Z => scale * Matrix4d::rotation_x(-FRAC_PI_2),
    }
}

fn canonical_up_axis_token() -> &'static str {
    match USDHUB_UP_AXIS {
        StageUpAxis::Y => "Y",
        StageUpAxis::Z => "Z",
    }
}

pub(crate) fn author_source_normalization(
    source_prim: &Prim,
    source: &SourceSpatialConvention,
) -> Result<()> {
    let transform = source_normalization_transform(source);
    source_prim
        .create_attribute("xformOp:transform", "matrix4d")?
        .set_custom(false)?
        .set(Value::Matrix4d(transform))?;
    source_prim
        .create_attribute("xformOpOrder", "token[]")?
        .set_custom(false)?
        .set(Value::TokenVec(vec!["xformOp:transform".into()]))?;
    Ok(())
}

/// Mark the wrapper's physical source anchor as transparent in USDHub's
/// semantic hierarchy. The marker is explicit metadata, so imported content
/// merely named `Source` or `Members` is never hidden by convention.
pub(crate) fn author_source_hierarchy_role(source_prim: &Prim) -> Result<()> {
    let mut custom_data = match source_prim.custom_data()? {
        Some(Value::Dictionary(data)) => data,
        _ => HashMap::new(),
    };
    custom_data.insert(
        USDHUB_HIERARCHY_ROLE_METADATA.to_owned(),
        Value::String(USDHUB_TRANSPARENT_SOURCE_ROLE.to_owned()),
    );
    source_prim
        .clone()
        .set_metadata("customData", Value::Dictionary(custom_data))?;
    Ok(())
}

/// Mark whether the Project-owned source wrapper has a machine-local linked
/// source binding. This provenance is canonical and contains no external
/// path, allowing a cloned Project to report a missing local binding without
/// confusing it with an ordinary imported Scene.
pub(crate) fn author_source_binding_role(source_prim: &Prim, linked: bool) -> Result<()> {
    let mut custom_data = match source_prim.custom_data()? {
        Some(Value::Dictionary(data)) => data,
        _ => HashMap::new(),
    };
    if linked {
        custom_data.insert(
            USDHUB_SOURCE_BINDING_METADATA.to_owned(),
            Value::String(USDHUB_LINKED_SOURCE_BINDING.to_owned()),
        );
    } else {
        custom_data.remove(USDHUB_SOURCE_BINDING_METADATA);
    }
    source_prim
        .clone()
        .set_metadata("customData", Value::Dictionary(custom_data))?;
    Ok(())
}

pub(crate) fn source_binding_is_linked(source_prim: &Prim) -> Result<bool> {
    Ok(source_binding_marker(source_prim)?.as_deref() == Some(USDHUB_LINKED_SOURCE_BINDING))
}

pub(crate) fn source_binding_marker(source_prim: &Prim) -> Result<Option<String>> {
    Ok(source_prim.custom_data()?.and_then(|value| match value {
        Value::Dictionary(data) => data
            .get(USDHUB_SOURCE_BINDING_METADATA)
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    }))
}

pub(crate) fn read_source_normalization(source_prim: &Prim) -> Result<Matrix4d> {
    let transform = source_prim
        .attribute("xformOp:transform")
        .get::<Value>()?
        .context("wrapper Source is missing xformOp:transform")?;
    let Value::Matrix4d(transform) = transform else {
        bail!("wrapper Source xformOp:transform must be matrix4d");
    };
    let order = source_prim
        .attribute("xformOpOrder")
        .get::<Value>()?
        .context("wrapper Source is missing xformOpOrder")?;
    ensure!(
        order == Value::TokenVec(vec!["xformOp:transform".into()]),
        "wrapper Source must use only xformOp:transform"
    );
    Ok(transform)
}

fn parse_up_axis(value: Value) -> Result<StageUpAxis> {
    let axis = value
        .as_str()
        .context("USD upAxis metadata must be a token or string")?;
    match axis {
        "Y" => Ok(StageUpAxis::Y),
        "Z" => Ok(StageUpAxis::Z),
        other => bail!("USD upAxis metadata has unsupported value {other:?}"),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/project/spatial_tests.rs"]
mod tests;
