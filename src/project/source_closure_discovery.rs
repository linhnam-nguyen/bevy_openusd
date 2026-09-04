use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use openusd::{
    ar::{DefaultResolver, Resolver, is_package_relative_path},
    sdf::Value,
    usd::{InitialLoadSet, PrimPredicate, Stage},
};

#[path = "source_closure_patterns.rs"]
mod patterns;
#[path = "source_closure_resolver.rs"]
mod resolver;
pub(crate) use patterns::expand_template_asset_paths;
pub(crate) use resolver::resolve_asset_paths_with_resolver;
use resolver::{SharedResolver, filesystem_identifier};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalizedDependencyReport {
    pub(crate) root_asset: PathBuf,
    pub(crate) layers: Vec<PathBuf>,
    pub(crate) non_layer_assets: Vec<PathBuf>,
    pub(crate) unresolved: Vec<String>,
    pub(crate) optional_unresolved: Vec<String>,
}

pub(crate) fn discover(source: &Path) -> Result<LocalizedDependencyReport> {
    let root_asset = regular_file(source).context("validate USD source")?;
    let resolver = DefaultResolver::with_search_paths([source.parent().unwrap_or(Path::new("."))]);
    discover_with_resolver(&root_asset, Arc::new(resolver))
}

/// Discover a closure through one configured OpenUSD resolver authority.
///
/// The same resolver is installed on every Stage opened during discovery and
/// is also used for asset-valued fields. This keeps resolver configuration and
/// resolver-context behavior at the OpenUSD boundary instead of reconstructing
/// filesystem candidates in USDHub.
pub(crate) fn discover_with_resolver(
    source: &Path,
    resolver: Arc<dyn Resolver>,
) -> Result<LocalizedDependencyReport> {
    let root_asset = regular_file(source).context("validate USD source")?;
    if is_usdz_path(&root_asset) {
        return discover_usdz(root_asset, resolver);
    }
    let mut state = DiscoveryState {
        root_asset: root_asset.clone(),
        layers: BTreeSet::new(),
        non_layer_assets: BTreeSet::new(),
        unresolved: BTreeSet::new(),
        optional_unresolved: BTreeSet::new(),
        pending: vec![root_asset.clone()],
        visited: BTreeSet::new(),
        scanned: BTreeSet::new(),
    };

    while let Some(layer_path) = state.pending.pop() {
        if !state.visited.insert(layer_path.clone()) {
            continue;
        }
        let stage = match open_stage_with_resolver(
            &layer_path,
            resolver.clone(),
            InitialLoadSet::LoadAll,
        ) {
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
            let Some(identifier_path) = filesystem_identifier(&identifier, resolver.as_ref())
            else {
                state
                    .unresolved
                    .insert(format!("{}: non-filesystem layer identifier", identifier));
                continue;
            };
            if is_package_relative_path(&identifier) || is_usdz_path(Path::new(&identifier)) {
                state.non_layer_assets.insert(identifier_path.clone());
            } else if state.add_layer(identifier_path.clone()).is_none() {
                continue;
            }
            if let Some(layer) = stage.layer(&identifier) {
                if state.scanned.insert(identifier.clone()) {
                    let anchor = if is_package_relative_path(&identifier)
                        || is_usdz_path(Path::new(&identifier))
                    {
                        PathBuf::from(&identifier)
                    } else {
                        identifier_path
                    };
                    scan_layer(&layer, &anchor, &mut state, resolver.as_ref())?;
                }
            }
        }
    }

    Ok(LocalizedDependencyReport {
        root_asset,
        layers: state.layers.into_iter().collect(),
        non_layer_assets: state.non_layer_assets.into_iter().collect(),
        unresolved: state.unresolved.into_iter().collect(),
        optional_unresolved: state.optional_unresolved.into_iter().collect(),
    })
}

fn discover_usdz(
    root_asset: PathBuf,
    resolver: Arc<dyn Resolver>,
) -> Result<LocalizedDependencyReport> {
    let mut unresolved = BTreeSet::new();
    match open_stage_with_resolver(&root_asset, resolver, InitialLoadSet::LoadNone) {
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
        optional_unresolved: Vec::new(),
    })
}

pub(crate) fn open_stage_with_resolver(
    path: &Path,
    resolver: Arc<dyn Resolver>,
    load: InitialLoadSet,
) -> Result<Stage> {
    let path_string = path
        .to_str()
        .context("USD dependency path must be valid UTF-8")?;
    Stage::builder()
        .resolver(SharedResolver(resolver))
        .load(load)
        .open(path_string)
        .with_context(|| format!("open USD stage {}", path.display()))
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

pub(crate) fn resolve_asset_path_with_resolver(
    resolver: &dyn Resolver,
    layer_path: &Path,
    authored: &str,
) -> Result<PathBuf> {
    let paths = resolve_asset_paths_with_resolver(resolver, layer_path, authored)?;
    ensure!(
        paths.len() == 1,
        "USD asset path resolves to multiple files and needs a pattern: {authored}"
    );
    Ok(paths.into_iter().next().expect("one resolved asset path"))
}

struct DiscoveryState {
    root_asset: PathBuf,
    layers: BTreeSet<PathBuf>,
    non_layer_assets: BTreeSet<PathBuf>,
    unresolved: BTreeSet<String>,
    optional_unresolved: BTreeSet<String>,
    pending: Vec<PathBuf>,
    visited: BTreeSet<PathBuf>,
    scanned: BTreeSet<String>,
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

    fn add_dependency(
        &mut self,
        layer_path: &Path,
        authored: &str,
        is_layer: bool,
        field: &str,
        resolver: &dyn Resolver,
    ) {
        match resolve_asset_paths_with_resolver(resolver, layer_path, authored) {
            Ok(paths) if is_layer => {
                for path in paths {
                    self.add_layer(path);
                }
            }
            Ok(paths) => {
                self.non_layer_assets.extend(paths);
            }
            Err(error) => {
                let message = format!("{} in {}: {error}", authored, layer_path.display());
                if !is_layer && is_optional_render_asset(authored) {
                    self.optional_unresolved
                        .insert(format!("{field}: {message}"));
                } else {
                    self.unresolved.insert(message);
                }
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

fn scan_layer(
    layer: &openusd::sdf::Layer,
    layer_path: &Path,
    state: &mut DiscoveryState,
    resolver: &dyn Resolver,
) -> Result<()> {
    let data = layer.data();
    for spec_path in data.spec_paths() {
        for field in data.list_fields(&spec_path).into_iter().flatten() {
            let Some(value) = data.try_field(&spec_path, &field)? else {
                continue;
            };
            scan_value(&value, layer_path, state, field.as_str(), resolver);
        }
    }
    Ok(())
}

fn scan_value(
    value: &Value,
    layer_path: &Path,
    state: &mut DiscoveryState,
    field: &str,
    resolver: &dyn Resolver,
) {
    match value {
        Value::AssetPath(asset) => {
            state.add_dependency(layer_path, asset.as_str(), false, field, resolver)
        }
        Value::AssetPathVec(assets) => {
            for asset in assets {
                state.add_dependency(layer_path, asset.as_str(), false, field, resolver);
            }
        }
        Value::ReferenceListOp(references) => {
            for reference in references.iter() {
                for value in reference.custom_data.values() {
                    scan_value(value, layer_path, state, "", resolver);
                }
            }
        }
        Value::PayloadListOp(_) => {}
        Value::Dictionary(values) => {
            if field == "clips" {
                scan_clip_dictionary(values, layer_path, state, resolver);
            } else {
                for value in values.values() {
                    scan_value(value, layer_path, state, "", resolver);
                }
            }
        }
        Value::ValueVec(values) => {
            for value in values {
                scan_value(value, layer_path, state, "", resolver);
            }
        }
        Value::TimeSamples(samples) => {
            for (_, value) in samples {
                scan_value(value, layer_path, state, "", resolver);
            }
        }
        _ => {}
    }
}

fn scan_clip_dictionary(
    values: &HashMap<String, Value>,
    layer_path: &Path,
    state: &mut DiscoveryState,
    resolver: &dyn Resolver,
) {
    for set in values.values().filter_map(|value| match value {
        Value::Dictionary(set) => Some(set),
        _ => None,
    }) {
        if let Some(Value::AssetPathVec(paths)) = set.get("assetPaths") {
            for path in paths {
                state.add_dependency(layer_path, path.as_str(), true, "clips", resolver);
            }
        }
        if let Some(Value::AssetPath(path)) = set.get("manifestAssetPath")
            && !path.is_empty()
        {
            state.add_dependency(layer_path, path.as_str(), true, "clips", resolver);
        }
        if let Some(Value::AssetPath(path)) = set.get("templateAssetPath") {
            match expand_template_asset_paths(set, path.as_str()) {
                Ok(paths) => {
                    for path in paths {
                        state.add_dependency(layer_path, &path, true, "clips", resolver);
                    }
                }
                Err(error) => {
                    state.unresolved.insert(format!(
                        "{} in {}: {error}",
                        path.as_str(),
                        layer_path.display()
                    ));
                }
            }
        }
    }
}

/// Missing renderer-only assets are retained as authored references and use
/// the runtime's deterministic flat/default material fallback. USD layers,
/// references, payloads, and clip files remain composition-critical.
pub(crate) fn is_optional_render_asset(authored: &str) -> bool {
    Path::new(authored)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mdl" | "png" | "jpg" | "jpeg" | "exr" | "hdr" | "tif" | "tiff" | "bmp" | "ktx2"
            )
        })
}
