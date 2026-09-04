//! Authored USD asset dependency discovery without composed-stage traversal.

use std::path::{Path, PathBuf};

use anyhow::Result;
use openusd::{sdf, usd::Stage};

use openusd::ar::split_package_relative_path_outer;

pub(super) fn authored_archive_paths(stage: &Stage) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for identifier in stage.layer_identifiers() {
        let Some(layer) = stage.layer(&identifier) else {
            continue;
        };
        let layer_base = split_package_relative_path_outer(&identifier)
            .map_or_else(|| identifier.clone(), |(outer, _)| outer);
        let Some(root) = layer.pseudo_root() else {
            continue;
        };
        for child in root.prim_children().unwrap_or_default() {
            let child_path = sdf::Path::abs_root().append_path(child.as_str())?;
            collect_prim(&layer, &child_path, &layer_base, &mut paths)?;
        }
    }
    Ok(paths)
}

fn collect_prim(
    layer: &sdf::Layer,
    path: &sdf::Path,
    layer_base: &str,
    paths: &mut Vec<PathBuf>,
) -> Result<()> {
    let Some(prim) = layer.prim(path.clone()) else {
        return Ok(());
    };
    for field in [
        sdf::FieldKey::References.as_str(),
        sdf::FieldKey::Payload.as_str(),
    ] {
        if let Some(value) = prim.field(field)? {
            collect_asset_paths(&value, layer_base, paths);
        }
    }
    for child in prim.prim_children().unwrap_or_default() {
        let child_path = path.append_path(child.as_str())?;
        collect_prim(layer, &child_path, layer_base, paths)?;
    }
    Ok(())
}

fn collect_asset_paths(value: &sdf::Value, layer_base: &str, paths: &mut Vec<PathBuf>) {
    match value {
        sdf::Value::ReferenceListOp(references) => {
            for reference in references.iter() {
                push_asset_path(layer_base, &reference.asset_path, paths);
            }
        }
        sdf::Value::Payload(payload) => push_asset_path(layer_base, &payload.asset_path, paths),
        sdf::Value::PayloadListOp(payloads) => {
            for payload in payloads.iter() {
                push_asset_path(layer_base, &payload.asset_path, paths);
            }
        }
        _ => {}
    }
}

fn push_asset_path(layer_base: &str, authored: &str, paths: &mut Vec<PathBuf>) {
    if authored.is_empty() {
        return;
    }
    let outer = split_package_relative_path_outer(authored)
        .map_or_else(|| authored.to_owned(), |(outer, _)| outer);
    let asset = Path::new(&outer);
    let layer = Path::new(layer_base);
    let resolved = if asset.is_absolute() {
        asset.to_path_buf()
    } else {
        layer.parent().unwrap_or_else(|| Path::new(".")).join(asset)
    };
    if resolved
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("usdz"))
    {
        paths.push(resolved);
    }
}
