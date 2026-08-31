use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail, ensure};
use openusd::{
    ar::{DefaultResolver, Resolver},
    usd::InitialLoadSet,
};

#[path = "source_closure_localize_patterns.rs"]
mod patterns;
#[path = "source_closure_localize_rewrite.rs"]
mod rewrite;
#[path = "source_closure_localize_validate.rs"]
mod validation;

use super::discovery::{
    LocalizedDependencyReport, discover, open_stage_with_resolver, resolve_asset_path_with_resolver,
};
use super::io::copy_file_synced;

/// Copy one import into a fresh Project-owned directory and return the copied
/// root source filename relative to that directory.
pub(crate) fn materialize_source_closure(source: &Path, destination: &Path) -> Result<String> {
    let root_asset = super::discovery::regular_file(source).context("validate USD source")?;
    let resolver =
        DefaultResolver::with_search_paths([root_asset.parent().unwrap_or(Path::new("."))]);
    materialize_source_closure_with_resolver(source, destination, Arc::new(resolver))
}

pub(crate) fn materialize_source_closure_with_resolver(
    source: &Path,
    destination: &Path,
    resolver: Arc<dyn Resolver>,
) -> Result<String> {
    let report = super::discovery::discover_with_resolver(source, resolver.clone())
        .context("discover USD source dependency closure")?;
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
        if is_usdz_root(&report.root_asset) && original == report.root_asset {
            // A USDZ package is already an atomic exact closure. Keeping the
            // archive intact preserves package-relative internal layer and
            // asset identifiers without copying neighboring filesystem data.
            copy_file_synced(&original, target)?;
        } else if layer_set.contains(&original) {
            localize_layer(&original, target, &mapping, resolver.clone())?;
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
    validation::validate_localized_root(localized_root, resolver)?;
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
    let parent = original
        .parent()
        .context("external USD dependency has no parent directory")?;
    let hash = blake3::hash(parent.to_string_lossy().as_bytes())
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
    let parent = original
        .parent()
        .context("external USD dependency has no parent directory")?;
    let hash = blake3::hash(parent.to_string_lossy().as_bytes())
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
    resolver: Arc<dyn Resolver>,
) -> Result<()> {
    let stage = open_stage_with_resolver(original, resolver.clone(), InitialLoadSet::LoadNone)
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
            let value = rewrite::rewrite_value(
                &value,
                original,
                destination,
                mapping,
                field.as_str(),
                resolver.as_ref(),
            )?;
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

fn is_usdz_root(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("usdz"))
}
