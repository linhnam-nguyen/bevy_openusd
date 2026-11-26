//! Canonical USDHub spatial metadata and source-wrapper normalization.

use std::path::Path;
use std::{collections::HashMap, f64::consts::FRAC_PI_2};

use anyhow::{Context, Result, bail, ensure};
use openusd::{vt, 
    gf::Mat4d,
    vt::Value,
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
        if let Some(layer) = stage.layer(&identifier)? {
            let borrowed = layer.borrow();
            if let Some(root) = borrowed.pseudo_root() {
                up_axis_was_authored |= root.has_field("upAxis");
                meters_per_unit_was_authored |= root.has_field("metersPerUnit");
            }
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
    let stage = Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .open(source)
        .context("open USD source for spatial inspection")?;
    inspect_stage(&stage)
}

/// Author the canonical metrics on a newly-created USDHub Stage.
pub(crate) fn author_canonical_stage(stage: &Stage) -> Result<()> {
    let root_identifier = stage.root_layer().identifier().to_owned();
    let root_layer = stage
        .layer(&root_identifier)?
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
pub(crate) fn source_normalization_transform(source: &SourceSpatialConvention) -> Mat4d {
    let unit_scale = source.meters_per_unit / USDHUB_METERS_PER_UNIT;
    let scale = Mat4d::scale([unit_scale, unit_scale, unit_scale]);
    match source.up_axis {
        StageUpAxis::Y => scale,
        StageUpAxis::Z => scale * Mat4d::rotation_x(-FRAC_PI_2),
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

const SOURCE_NORMALIZATION_OP: &str = "xformOp:transform:usdhubNormalization";

/// Preserve a single referenced root's uniform authored operation order and
/// prepend a wrapper-owned normalization operation to its active stack. Keep
/// the existing Source occurrence path and leave source operation values and
/// time samples referenced, never copied. Multi-root wrappers normalize their
/// separate Source parent as before.
pub(crate) fn author_referenced_source_normalization(
    source_prim: &Prim,
    source: &SourceSpatialConvention,
    source_metadata_path: &Path,
    source_prims: &[String],
) -> Result<()> {
    if source_prims.len() != 1 {
        return author_source_normalization(source_prim, source);
    }
    let stage = Stage::builder().load(openusd::usd::InitialLoadSet::LoadNone)
        .open(source_metadata_path.to_str().context("source metadata path is not UTF-8")?)?;
    let prim = stage.prim(source_prims[0].as_str())?;
    let source_order = prim.attribute("xformOpOrder").get::<Value>()?;
    if source_order.is_some() {
        let order_path = openusd::sdf::path(format!("{}.xformOpOrder", source_prims[0]))?;
        ensure!(stage.time_sample_times(order_path)?.is_none_or(|times| times.is_empty()),
            "time-varying xformOpOrder cannot be normalized without changing source semantics");
    }
    let mut order: Vec<String> = match source_order {
        None => Vec::new(),
        Some(Value::TokenVec(tokens)) => tokens.into_iter().map(|token| token.as_str().to_owned()).collect(),
        Some(Value::StringVec(tokens)) => tokens,
        Some(Value::TokenListOp(tokens)) => tokens.flatten().into_iter().map(|token| token.as_str().to_owned()).collect(),
        Some(_) => bail!("unsupported source xformOpOrder"),
    };
    // Legacy/static root layout remains unchanged where no source transform
    // opinion exists. Never reuse an operation name already owned by a source.
    if order.is_empty() && prim.attribute("xformOp:transform").get::<Value>()?.is_none() {
        return author_source_normalization(source_prim, source);
    }
    ensure!(!order.iter().any(|op| op == SOURCE_NORMALIZATION_OP || op.strip_prefix("!invert!") == Some(SOURCE_NORMALIZATION_OP))
        && prim.attribute(SOURCE_NORMALIZATION_OP).get::<Value>()?.is_none(),
        "source already owns reserved wrapper normalization operation");
    let active_start = order.iter().rposition(|op| op == "!resetXformStack!").map_or(0, |index| index + 1);
    order.insert(active_start, SOURCE_NORMALIZATION_OP.to_owned());
    source_prim.create_attribute(SOURCE_NORMALIZATION_OP, "matrix4d")?
        .set_custom(false)?.set(Value::Matrix4d(source_normalization_transform(source)))?;
    source_prim.create_attribute("xformOpOrder", "token[]")?.set_custom(false)?
        .set(Value::TokenVec(order.into_iter().map(Into::into).collect()))?;
    Ok(())
}

/// Mark the wrapper's physical source anchor as transparent in USDHub's
/// semantic hierarchy. The marker is explicit metadata, so imported content
/// merely named `Source` or `Members` is never hidden by convention.
pub(crate) fn author_source_hierarchy_role(source_prim: &Prim) -> Result<()> {
    let mut custom_data = match source_prim.metadata::<Value>("customData")? {
        Some(Value::Dictionary(data)) => data,
        _ => openusd::vt::Dictionary::new(),
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
    let mut custom_data = match source_prim.metadata::<Value>("customData")? {
        Some(Value::Dictionary(data)) => data,
        _ => openusd::vt::Dictionary::new(),
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
    Ok(source_prim
        .metadata::<Value>("customData")?
        .and_then(|value| match value {
            Value::Dictionary(data) => data
                .get(USDHUB_SOURCE_BINDING_METADATA)
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        }))
}

pub(crate) fn read_source_normalization(source_prim: &Prim) -> Result<Mat4d> {
    let order = source_prim.attribute("xformOpOrder").get::<Value>()?
        .context("wrapper Source is missing xformOpOrder")?;
    let Value::TokenVec(order) = order else {
        bail!("wrapper Source xformOpOrder must be token[]");
    };
    let active_start = order.iter().rposition(|op| op.as_str() == "!resetXformStack!")
        .map_or(0, |index| index + 1);
    let active = &order[active_start..];
    let operation = if active.len() == 1 && active[0].as_str() == "xformOp:transform" {
        "xformOp:transform"
    } else {
        ensure!(active.first().is_some_and(|op| op.as_str() == SOURCE_NORMALIZATION_OP)
            && active.iter().filter(|op| op.as_str() == SOURCE_NORMALIZATION_OP).count() == 1,
            "wrapper normalization must precede the active source transform stack");
        SOURCE_NORMALIZATION_OP
    };
    let transform = source_prim.attribute(operation).get::<Value>()?
        .context("wrapper Source is missing normalization transform")?;
    let Value::Matrix4d(transform) = transform else {
        bail!("wrapper normalization transform must be matrix4d");
    };
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
#[path = "../../../tests/unit/project/assets/spatial_tests.rs"]
mod tests;
