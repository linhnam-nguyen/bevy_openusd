use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use openusd::{vt, 
    ar::{Resolver, join_package_relative_path, split_package_relative_path_outer},
    sdf,
};

use super::super::discovery::{is_optional_render_asset, resolve_asset_path_with_resolver};
use super::patterns;

pub(super) fn rewrite_value(
    value: &vt::Value,
    original_layer: &Path,
    localized_layer: &Path,
    mapping: &BTreeMap<PathBuf, PathBuf>,
    field: &str,
    resolver: &dyn Resolver,
) -> Result<vt::Value> {
    let mut rewritten = value.clone();
    match &mut rewritten {
        vt::Value::AssetPath(asset) => {
            rewrite_asset(asset, original_layer, localized_layer, mapping, resolver)?
        }
        vt::Value::AssetPathVec(assets) => {
            for asset in assets {
                rewrite_asset(asset, original_layer, localized_layer, mapping, resolver)?;
            }
        }
        vt::Value::ReferenceListOp(references) => {
            let mut err: Result<()> = Ok(());
            references.update(|reference| {
                if err.is_err() {
                    return;
                }
                if !reference.asset_path.is_empty() {
                    match layer_asset(
                        &reference.asset_path,
                        original_layer,
                        localized_layer,
                        mapping,
                        resolver,
                    ) {
                        Ok(rewritten) => reference.asset_path = rewritten,
                        Err(e) => {
                            err = Err(e);
                            return;
                        }
                    }
                }
                for value in reference.custom_data.values_mut() {
                    match rewrite_value(
                        value,
                        original_layer,
                        localized_layer,
                        mapping,
                        "",
                        resolver,
                    ) {
                        Ok(rewritten) => *value = rewritten,
                        Err(e) => {
                            err = Err(e);
                            return;
                        }
                    }
                }
            });
            err?;
        }
        vt::Value::PayloadListOp(payloads) => {
            let mut err: Result<()> = Ok(());
            payloads.update(|payload| {
                if err.is_err() {
                    return;
                }
                if !payload.asset_path.is_empty() {
                    match layer_asset(
                        &payload.asset_path,
                        original_layer,
                        localized_layer,
                        mapping,
                        resolver,
                    ) {
                        Ok(rewritten) => payload.asset_path = rewritten,
                        Err(e) => {
                            err = Err(e);
                        }
                    }
                }
            });
            err?;
        }
        vt::Value::Payload(payload) => {
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
        vt::Value::Dictionary(values) => {
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
        vt::Value::ValueVec(values) => {
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
        vt::Value::TimeSamples(samples) => {
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
        vt::Value::StringVec(paths) if field == sdf::FieldKey::SubLayers.as_str() => {
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
        let authored = asset.authored_path.clone();
        match patterns::rewrite_asset_path(
            &authored,
            original_layer,
            localized_layer,
            mapping,
            resolver,
        ) {
            Ok(rewritten) => asset.authored_path = rewritten,
            Err(error) if is_optional_render_asset(&authored) => {
                log::debug!(
                    "retaining missing optional renderer asset {authored} in {}: {error:#}",
                    original_layer.display()
                );
            }
            Err(error) => return Err(error),
        }
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
