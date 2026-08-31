use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use openusd::ar::Resolver;
use openusd::sdf;

use super::super::discovery;

pub(super) fn rewrite_asset_path(
    authored: &str,
    original_layer: &Path,
    localized_layer: &Path,
    mapping: &BTreeMap<PathBuf, PathBuf>,
    resolver: &dyn Resolver,
) -> Result<String> {
    let original_assets =
        discovery::resolve_asset_paths_with_resolver(resolver, original_layer, authored)?;
    if original_assets.len() == 1 {
        return super::rewrite::layer_asset(
            authored,
            original_layer,
            localized_layer,
            mapping,
            resolver,
        );
    }
    rewrite_pattern_asset(authored, localized_layer, mapping, &original_assets)
}

fn rewrite_pattern_asset(
    authored: &str,
    localized_layer: &Path,
    mapping: &BTreeMap<PathBuf, PathBuf>,
    original_assets: &[PathBuf],
) -> Result<String> {
    let first = original_assets
        .first()
        .context("pattern asset has no resolved files")?;
    let first_target = mapping.get(first).with_context(|| {
        format!(
            "USD pattern asset is outside exact closure: {}",
            first.display()
        )
    })?;
    let target_parent = first_target
        .parent()
        .context("localized pattern asset has no parent directory")?;
    for original in original_assets {
        let target = mapping.get(original).with_context(|| {
            format!(
                "USD pattern asset is outside exact closure: {}",
                original.display()
            )
        })?;
        ensure!(
            target.parent() == Some(target_parent),
            "USD pattern assets must localize into one directory: {authored}"
        );
    }
    let file_name = Path::new(authored)
        .file_name()
        .context("pattern asset has no filename")?;
    let localized_pattern = target_parent.join(file_name);
    crate::project::storage::authored_relative_asset_path(localized_layer, &localized_pattern)
}

pub(super) fn rewrite_clip_dictionary(
    values: &mut std::collections::HashMap<String, sdf::Value>,
    original_layer: &Path,
    localized_layer: &Path,
    mapping: &BTreeMap<PathBuf, PathBuf>,
    resolver: &dyn Resolver,
) -> Result<()> {
    for set in values.values_mut().filter_map(|value| match value {
        sdf::Value::Dictionary(set) => Some(set),
        _ => None,
    }) {
        if let Some(sdf::Value::AssetPathVec(paths)) = set.get_mut("assetPaths") {
            for path in paths {
                if !path.is_empty() {
                    path.authored_path = super::rewrite::layer_asset(
                        &path.authored_path,
                        original_layer,
                        localized_layer,
                        mapping,
                        resolver,
                    )?;
                }
            }
        }
        if let Some(sdf::Value::AssetPath(path)) = set.get_mut("manifestAssetPath")
            && !path.is_empty()
        {
            path.authored_path = super::rewrite::layer_asset(
                &path.authored_path,
                original_layer,
                localized_layer,
                mapping,
                resolver,
            )?;
        }
        let template = match set.get("templateAssetPath") {
            Some(sdf::Value::AssetPath(path)) if !path.is_empty() => {
                Some(path.authored_path.clone())
            }
            _ => None,
        };
        if let Some(template) = template {
            let expanded = discovery::expand_template_asset_paths(set, &template)?;
            let originals = expanded
                .iter()
                .map(|asset| {
                    discovery::resolve_asset_path_with_resolver(resolver, original_layer, asset)
                })
                .collect::<Result<Vec<_>>>()?;
            let rewritten = rewrite_pattern_asset(&template, localized_layer, mapping, &originals)?;
            if let Some(sdf::Value::AssetPath(path)) = set.get_mut("templateAssetPath") {
                path.authored_path = rewritten;
            }
        }
    }
    Ok(())
}
