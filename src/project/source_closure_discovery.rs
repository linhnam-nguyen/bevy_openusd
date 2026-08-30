use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use openusd::{
    sdf::Value,
    usd::{InitialLoadSet, PrimPredicate, Stage},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalizedDependencyReport {
    pub(crate) root_asset: PathBuf,
    pub(crate) layers: Vec<PathBuf>,
    pub(crate) non_layer_assets: Vec<PathBuf>,
    pub(crate) unresolved: Vec<String>,
}

pub(crate) fn discover(source: &Path) -> Result<LocalizedDependencyReport> {
    let root_asset = regular_file(source).context("validate USD source")?;
    if is_usdz_path(&root_asset) {
        return discover_usdz(root_asset);
    }
    let mut state = DiscoveryState {
        root_asset: root_asset.clone(),
        layers: BTreeSet::new(),
        non_layer_assets: BTreeSet::new(),
        unresolved: BTreeSet::new(),
        pending: vec![root_asset.clone()],
        visited: BTreeSet::new(),
    };

    while let Some(layer_path) = state.pending.pop() {
        if !state.visited.insert(layer_path.clone()) {
            continue;
        }
        let path_string = layer_path
            .to_str()
            .context("USD dependency path must be valid UTF-8")?;
        let stage = match Stage::builder()
            .load(InitialLoadSet::LoadNone)
            .open(path_string)
        {
            Ok(stage) => stage,
            Err(error) => {
                state.unresolved.insert(format!(
                    "{}: unable to open dependency: {error}",
                    layer_path.display()
                ));
                continue;
            }
        };
        force_reachable_layers(&stage)?;
        for error in stage.composition_errors() {
            state.unresolved.insert(format!("composition: {error}"));
        }
        for identifier in stage.layer_identifiers() {
            let Some(identifier_path) = filesystem_identifier(&identifier) else {
                state
                    .unresolved
                    .insert(format!("{}: non-filesystem layer identifier", identifier));
                continue;
            };
            let Some(identifier_path) = state.add_layer(identifier_path) else {
                continue;
            };
            if let Some(layer) = stage.layer(&identifier) {
                scan_layer(&layer, &identifier_path, &mut state)?;
            }
        }
    }

    Ok(LocalizedDependencyReport {
        root_asset,
        layers: state.layers.into_iter().collect(),
        non_layer_assets: state.non_layer_assets.into_iter().collect(),
        unresolved: state.unresolved.into_iter().collect(),
    })
}

fn discover_usdz(root_asset: PathBuf) -> Result<LocalizedDependencyReport> {
    let mut unresolved = BTreeSet::new();
    let path_string = root_asset
        .to_str()
        .context("USDZ dependency path must be valid UTF-8")?;
    match Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(path_string)
    {
        Ok(stage) => {
            if let Err(error) = force_reachable_layers(&stage) {
                unresolved.insert(format!(
                    "{}: unable to traverse package: {error}",
                    root_asset.display()
                ));
            }
            for error in stage.composition_errors() {
                unresolved.insert(format!("composition: {error}"));
            }
        }
        Err(error) => {
            unresolved.insert(format!(
                "{}: unable to open dependency: {error}",
                root_asset.display()
            ));
        }
    }
    Ok(LocalizedDependencyReport {
        root_asset,
        layers: Vec::new(),
        non_layer_assets: Vec::new(),
        unresolved: unresolved.into_iter().collect(),
    })
}

fn is_usdz_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("usdz"))
}

pub(crate) fn regular_file(path: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("read USD dependency metadata {}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "USD dependency must be a regular non-symlink file: {}",
        path.display()
    );
    fs::canonicalize(path)
        .with_context(|| format!("canonicalize USD dependency {}", path.display()))
}

pub(crate) fn resolve_asset_path(layer_path: &Path, authored: &str) -> Result<PathBuf> {
    if authored.is_empty() {
        bail!("USD asset path is empty");
    }
    if authored.starts_with("anon:") || authored.contains('[') {
        bail!("USD asset path is not a filesystem path: {authored}");
    }
    let authored_path = Path::new(authored);
    let candidate = if authored_path.is_absolute() {
        authored_path.to_owned()
    } else {
        layer_path
            .parent()
            .context("USD dependency layer has no parent directory")?
            .join(authored_path)
    };
    regular_file(&candidate)
}

struct DiscoveryState {
    root_asset: PathBuf,
    layers: BTreeSet<PathBuf>,
    non_layer_assets: BTreeSet<PathBuf>,
    unresolved: BTreeSet<String>,
    pending: Vec<PathBuf>,
    visited: BTreeSet<PathBuf>,
}

impl DiscoveryState {
    fn add_layer(&mut self, path: PathBuf) -> Option<PathBuf> {
        let path = match regular_file(&path) {
            Ok(path) => path,
            Err(error) => {
                self.unresolved
                    .insert(format!("{}: {error}", path.display()));
                return None;
            }
        };
        if path != self.root_asset {
            self.layers.insert(path.clone());
        }
        if !self.visited.contains(&path) {
            self.pending.push(path.clone());
        }
        Some(path)
    }

    fn add_dependency(&mut self, layer_path: &Path, authored: &str, is_layer: bool) {
        match resolve_asset_path(layer_path, authored) {
            Ok(path) if is_layer => {
                self.add_layer(path);
            }
            Ok(path) => {
                self.non_layer_assets.insert(path);
            }
            Err(error) => {
                self.unresolved.insert(format!(
                    "{} in {}: {error}",
                    authored,
                    layer_path.display()
                ));
            }
        }
    }
}

fn force_reachable_layers(stage: &Stage) -> Result<()> {
    stage.traverse(PrimPredicate::ALL, |path| {
        let prim = stage.prim(path);
        let _ = prim.prim_index().graph();
    })?;
    Ok(())
}

fn filesystem_identifier(identifier: &str) -> Option<PathBuf> {
    (!identifier.starts_with("anon:") && !identifier.contains('['))
        .then(|| PathBuf::from(identifier))
}

fn scan_layer(
    layer: &openusd::sdf::Layer,
    layer_path: &Path,
    state: &mut DiscoveryState,
) -> Result<()> {
    let data = layer.data();
    for spec_path in data.spec_paths() {
        for field in data.list_fields(&spec_path).into_iter().flatten() {
            let Some(value) = data.try_field(&spec_path, &field)? else {
                continue;
            };
            if field == openusd::sdf::FieldKey::SubLayers.as_str() {
                if let Value::StringVec(paths) = &*value {
                    for path in paths {
                        state.add_dependency(layer_path, path, true);
                    }
                }
            }
            scan_value(&value, layer_path, state);
        }
    }
    Ok(())
}

fn scan_value(value: &Value, layer_path: &Path, state: &mut DiscoveryState) {
    match value {
        Value::AssetPath(asset) => state.add_dependency(layer_path, asset.as_str(), false),
        Value::AssetPathVec(assets) => {
            for asset in assets {
                state.add_dependency(layer_path, asset.as_str(), false);
            }
        }
        Value::ReferenceListOp(references) => {
            for reference in references.iter() {
                if !reference.asset_path.is_empty() {
                    state.add_dependency(layer_path, &reference.asset_path, true);
                }
                for value in reference.custom_data.values() {
                    scan_value(value, layer_path, state);
                }
            }
        }
        Value::PayloadListOp(payloads) => {
            for payload in payloads.iter() {
                if !payload.asset_path.is_empty() {
                    state.add_dependency(layer_path, &payload.asset_path, true);
                }
            }
        }
        Value::Payload(payload) => {
            if !payload.asset_path.is_empty() {
                state.add_dependency(layer_path, &payload.asset_path, true);
            }
        }
        Value::Dictionary(values) => {
            for value in values.values() {
                scan_value(value, layer_path, state);
            }
        }
        Value::ValueVec(values) => {
            for value in values {
                scan_value(value, layer_path, state);
            }
        }
        Value::TimeSamples(samples) => {
            for (_, value) in samples {
                scan_value(value, layer_path, state);
            }
        }
        _ => {}
    }
}
