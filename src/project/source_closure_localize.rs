use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use openusd::{
    sdf,
    usd::{InitialLoadSet, PrimPredicate, Stage},
};

use super::discovery::{LocalizedDependencyReport, discover, resolve_asset_path};
use super::io::copy_file_synced;

/// Copy one import into a fresh Project-owned directory and return the copied
/// root source filename relative to that directory.
pub(crate) fn materialize_source_closure(source: &Path, destination: &Path) -> Result<String> {
    let report = discover(source).context("discover USD source dependency closure")?;
    ensure_resolved(&report)?;
    ensure!(
        !destination.exists(),
        "source-closure destination already exists"
    );

    let files = closure_files(&report);
    let mapping = build_mapping(&report, destination)?;
    let layer_set = report
        .layers
        .iter()
        .chain(std::iter::once(&report.root_asset))
        .cloned()
        .collect::<BTreeSet<_>>();
    fs::create_dir_all(destination)
        .with_context(|| format!("create source closure {}", destination.display()))?;

    for original in files {
        let target = mapping
            .get(&original)
            .with_context(|| format!("missing localization target for {}", original.display()))?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create source closure {}", parent.display()))?;
        }
        if layer_set.contains(&original) {
            localize_layer(&original, target, &mapping)?;
        } else {
            copy_file_synced(&original, target)?;
        }
    }

    let source_name = report
        .root_asset
        .file_name()
        .and_then(|name| name.to_str())
        .context("USD source filename must be valid UTF-8")?
        .to_owned();
    let localized_root = mapping
        .get(&report.root_asset)
        .context("localized USD root is missing")?;
    ensure!(
        localized_root.is_file(),
        "source closure did not materialize its root source"
    );
    validate_localized_root(localized_root)?;
    Ok(source_name)
}

/// Fingerprint only the exact transitive closure used by the source, excluding
/// unrelated neighboring files from the source directory.
pub(crate) fn source_closure_fingerprint(source: &Path) -> Result<String> {
    let report = discover(source).context("discover USD source dependency closure")?;
    ensure_resolved(&report)?;
    let source_parent = report
        .root_asset
        .parent()
        .context("USD source has no parent directory")?;
    let mapping = logical_mapping(&report, source_parent)?;
    let mut entries = mapping.into_iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.1.cmp(&right.1));

    let mut hasher = blake3::Hasher::new();
    hasher.update(b"USDHub source closure fingerprint v2\0");
    for (path, logical) in entries {
        let bytes = fs::read(&path)
            .with_context(|| format!("read source closure file {}", path.display()))?;
        hasher.update(&(logical.len() as u64).to_le_bytes());
        hasher.update(logical.as_bytes());
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn ensure_resolved(report: &LocalizedDependencyReport) -> Result<()> {
    if report.unresolved.is_empty() {
        return Ok(());
    }
    bail!(
        "USD source dependency closure has unresolved required dependencies: {}",
        report.unresolved.join("; ")
    )
}

fn closure_files(report: &LocalizedDependencyReport) -> Vec<PathBuf> {
    report
        .layers
        .iter()
        .chain(std::iter::once(&report.root_asset))
        .chain(report.non_layer_assets.iter())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn build_mapping(
    report: &LocalizedDependencyReport,
    destination: &Path,
) -> Result<BTreeMap<PathBuf, PathBuf>> {
    let source_parent = report
        .root_asset
        .parent()
        .context("USD source has no parent directory")?;
    let mut mapping = BTreeMap::new();
    for original in closure_files(report) {
        let target = localized_path(&original, source_parent, destination)?;
        ensure!(
            mapping.insert(original.clone(), target).is_none(),
            "duplicate USD source closure path {}",
            original.display()
        );
    }
    Ok(mapping)
}

fn logical_mapping(
    report: &LocalizedDependencyReport,
    source_parent: &Path,
) -> Result<BTreeMap<PathBuf, String>> {
    let mut mapping = BTreeMap::new();
    for original in closure_files(report) {
        let logical = logical_path(&original, source_parent)?;
        mapping.insert(original, logical);
    }
    Ok(mapping)
}

fn localized_path(original: &Path, source_parent: &Path, destination: &Path) -> Result<PathBuf> {
    if let Ok(relative) = original.strip_prefix(source_parent) {
        return Ok(destination.join(relative));
    }
    let hash = blake3::hash(original.to_string_lossy().as_bytes())
        .to_hex()
        .to_string();
    let name = original
        .file_name()
        .and_then(|name| name.to_str())
        .context("USD dependency filename must be valid UTF-8")?;
    Ok(destination.join("external").join(&hash[..16]).join(name))
}

fn logical_path(original: &Path, source_parent: &Path) -> Result<String> {
    if let Ok(relative) = original.strip_prefix(source_parent) {
        return Ok(format!(
            "source/{}",
            relative.to_string_lossy().replace('\\', "/")
        ));
    }
    let hash = blake3::hash(original.to_string_lossy().as_bytes())
        .to_hex()
        .to_string();
    let name = original
        .file_name()
        .and_then(|name| name.to_str())
        .context("USD dependency filename must be valid UTF-8")?;
    Ok(format!("external/{hash}/{name}"))
}

fn localize_layer(
    original: &Path,
    destination: &Path,
    mapping: &BTreeMap<PathBuf, PathBuf>,
) -> Result<()> {
    let source_string = original
        .to_str()
        .context("USD layer path must be valid UTF-8")?;
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(source_string)
        .with_context(|| format!("open USD layer {} for localization", original.display()))?;
    let root_identifier = stage.root_layer().identifier().to_owned();
    let mut layer = stage
        .layer_mut(&root_identifier)
        .with_context(|| format!("find USD layer {} for localization", original.display()))?;

    let records = {
        let data = layer.data();
        let mut records = Vec::new();
        for spec_path in data.spec_paths() {
            for field in data.list_fields(&spec_path).into_iter().flatten() {
                let Some(value) = data.try_field(&spec_path, &field)? else {
                    continue;
                };
                records.push((spec_path.clone(), field, value.into_owned()));
            }
        }
        records
    };
    let rewritten = records
        .into_iter()
        .map(|(path, field, value)| {
            let value = rewrite_value(&value, original, destination, mapping, &field)?;
            Ok((path, field, value))
        })
        .collect::<Result<Vec<_>>>()?;

    layer
        .edit(|edit| {
            for (path, field, value) in rewritten {
                edit.data_mut().set_field(&path, &field, value);
            }
            Ok(())
        })
        .with_context(|| format!("rewrite USD layer {}", original.display()))?;
    layer
        .export(
            destination
                .to_str()
                .context("localized USD layer path must be valid UTF-8")?,
        )
        .with_context(|| format!("export localized USD layer {}", destination.display()))
}

fn rewrite_value(
    value: &sdf::Value,
    original_layer: &Path,
    localized_layer: &Path,
    mapping: &BTreeMap<PathBuf, PathBuf>,
    field: &str,
) -> Result<sdf::Value> {
    let mut rewritten = value.clone();
    match &mut rewritten {
        sdf::Value::AssetPath(asset) => {
            rewrite_asset(asset, original_layer, localized_layer, mapping)?
        }
        sdf::Value::AssetPathVec(assets) => {
            for asset in assets {
                rewrite_asset(asset, original_layer, localized_layer, mapping)?;
            }
        }
        sdf::Value::ReferenceListOp(references) => {
            for reference in references.iter_mut() {
                if !reference.asset_path.is_empty() {
                    reference.asset_path = rewrite_layer_asset(
                        &reference.asset_path,
                        original_layer,
                        localized_layer,
                        mapping,
                    )?;
                }
                for value in reference.custom_data.values_mut() {
                    *value = rewrite_value(value, original_layer, localized_layer, mapping, "")?;
                }
            }
        }
        sdf::Value::PayloadListOp(payloads) => {
            for payload in payloads.iter_mut() {
                if !payload.asset_path.is_empty() {
                    payload.asset_path = rewrite_layer_asset(
                        &payload.asset_path,
                        original_layer,
                        localized_layer,
                        mapping,
                    )?;
                }
            }
        }
        sdf::Value::Payload(payload) => {
            if !payload.asset_path.is_empty() {
                payload.asset_path = rewrite_layer_asset(
                    &payload.asset_path,
                    original_layer,
                    localized_layer,
                    mapping,
                )?;
            }
        }
        sdf::Value::Dictionary(values) => {
            for value in values.values_mut() {
                *value = rewrite_value(value, original_layer, localized_layer, mapping, "")?;
            }
        }
        sdf::Value::ValueVec(values) => {
            for value in values {
                *value = rewrite_value(value, original_layer, localized_layer, mapping, "")?;
            }
        }
        sdf::Value::TimeSamples(samples) => {
            for (_, value) in samples {
                *value = rewrite_value(value, original_layer, localized_layer, mapping, "")?;
            }
        }
        sdf::Value::StringVec(paths) if field == sdf::FieldKey::SubLayers.as_str() => {
            for path in paths {
                *path = rewrite_layer_asset(path, original_layer, localized_layer, mapping)?;
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
) -> Result<()> {
    if !asset.is_empty() {
        asset.authored_path = rewrite_layer_asset(
            &asset.authored_path,
            original_layer,
            localized_layer,
            mapping,
        )?;
    }
    Ok(())
}

fn rewrite_layer_asset(
    authored: &str,
    original_layer: &Path,
    localized_layer: &Path,
    mapping: &BTreeMap<PathBuf, PathBuf>,
) -> Result<String> {
    let original_asset = resolve_asset_path(original_layer, authored).with_context(|| {
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
    crate::project::storage::authored_relative_asset_path(localized_layer, localized_asset)
}

fn validate_localized_root(path: &Path) -> Result<()> {
    let path_string = path
        .to_str()
        .context("localized USD root path must be valid UTF-8")?;
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(path_string)
        .context("reopen localized USD root")?;
    stage
        .traverse(PrimPredicate::ALL, |_| {})
        .context("traverse localized USD root")?;
    ensure!(
        stage.composition_errors().is_empty(),
        "localized USD root has composition errors"
    );
    let inspection = crate::project::scene::inspection::inspect_composition(path)
        .context("reinspect localized USD root")?;
    ensure!(
        !matches!(
            inspection.classification,
            usd_project::CompositionClassification::Unsupported
        ),
        "localized USD root failed composition inspection: {:?}",
        inspection.diagnostics
    );
    Ok(())
}
