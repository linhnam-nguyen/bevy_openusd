use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use openusd::{
    ar::{Resolver, join_package_relative_path, split_package_relative_path_outer},
    sdf,
};

use super::super::discovery::resolve_asset_path_with_resolver;
use super::patterns;

pub(super) fn rewrite_value(
    value: &sdf::Value,
    original_layer: &Path,
    localized_layer: &Path,
    mapping: &BTreeMap<PathBuf, PathBuf>,
    field: &str,
    resolver: &dyn Resolver,
) -> Result<sdf::Value> {
    let mut rewritten = value.clone();
    match &mut rewritten {
        sdf::Value::AssetPath(asset) => {
            rewrite_asset(asset, original_layer, localized_layer, mapping, resolver)?
        }
        sdf::Value::AssetPathVec(assets) => {
            for asset in assets {
                rewrite_asset(asset, original_layer, localized_layer, mapping, resolver)?;
            }
        }
        sdf::Value::ReferenceListOp(references) => {
            for reference in references.iter_mut() {
                if !reference.asset_path.is_empty() {
                    reference.asset_path = layer_asset(
                        &reference.asset_path,
                        original_layer,
                        localized_layer,
                        mapping,
                        resolver,
                    )?;
                }
                for value in reference.custom_data.values_mut() {
                    *value = rewrite_value(
                        value,
                        original_layer,
                        localized_layer,
                        mapping,
                        "",
                        resolver,
                    )?;
                }
            }
        }
        sdf::Value::PayloadListOp(payloads) => {
            for payload in payloads.iter_mut() {
                if !payload.asset_path.is_empty() {
                    payload.asset_path = layer_asset(
                        &payload.asset_path,
                        original_layer,
                        localized_layer,
                        mapping,
                        resolver,
                    )?;
                }
            }
        }
        sdf::Value::Payload(payload) => {
            if !payload.asset_path.is_empty() {
                payload.asset_path = layer_asset(
                    &payload.asset_path,
                    original_layer,
                    localized_layer,
                    mapping,
                    resolver,
                )?;
            }
        }
        sdf::Value::Dictionary(values) => {
            if field == "clips" {
                patterns::rewrite_clip_dictionary(
                    values,
                    original_layer,
                    localized_layer,
                    mapping,
                    resolver,
                )?;
            } else {
                for value in values.values_mut() {
                    *value = rewrite_value(
                        value,
                        original_layer,
                        localized_layer,
                        mapping,
                        "",
                        resolver,
                    )?;
                }
            }
        }
        sdf::Value::ValueVec(values) => {
            for value in values {
                *value = rewrite_value(
                    value,
                    original_layer,
                    localized_layer,
                    mapping,
                    "",
                    resolver,
                )?;
            }
        }
        sdf::Value::TimeSamples(samples) => {
            for (_, value) in samples {
                *value = rewrite_value(
                    value,
                    original_layer,
                    localized_layer,
                    mapping,
                    "",
                    resolver,
                )?;
            }
        }
        sdf::Value::StringVec(paths) if field == sdf::FieldKey::SubLayers.as_str() => {
            for path in paths {
                *path = layer_asset(path, original_layer, localized_layer, mapping, resolver)?;
            }
        }
        _ => {}
    }
    Ok(rewritten)
}

fn rewrite_asset(
    asset: &mut sdf::AssetPath,
    original_layer: &Path,
    localized_layer: &Path,
    mapping: &BTreeMap<PathBuf, PathBuf>,
    resolver: &dyn Resolver,
) -> Result<()> {
    if !asset.is_empty() {
        asset.authored_path = patterns::rewrite_asset_path(
            &asset.authored_path,
            original_layer,
            localized_layer,
            mapping,
            resolver,
        )?;
    }
    Ok(())
}

pub(super) fn layer_asset(
    authored: &str,
    original_layer: &Path,
    localized_layer: &Path,
    mapping: &BTreeMap<PathBuf, PathBuf>,
    resolver: &dyn Resolver,
) -> Result<String> {
    let original_asset = resolve_asset_path_with_resolver(resolver, original_layer, authored)
        .with_context(|| {
            format!(
                "resolve USD asset {authored} in {}",
                original_layer.display()
            )
        })?;
    let localized_asset = mapping.get(&original_asset).with_context(|| {
        format!(
            "USD asset is outside exact closure: {}",
            original_asset.display()
        )
    })?;
    let localized_path =
        crate::project::storage::authored_relative_asset_path(localized_layer, localized_asset)?;
    if let Some((_, packaged_path)) = split_package_relative_path_outer(authored) {
        Ok(join_package_relative_path(&localized_path, &packaged_path))
    } else {
        Ok(localized_path)
    }
}
