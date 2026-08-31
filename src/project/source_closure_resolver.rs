use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail};
use openusd::ar::{
    ResolvedPath, Resolver, is_package_relative_path, split_package_relative_path_outer,
};

/// Share one configured resolver between the Stage and the value scanner.
///
/// The pinned OpenUSD resolver API is trait-based and stages take ownership of
/// their resolver. This adapter keeps that one authority cloneable for the
/// discovery loop while preserving the resolver's configured search behavior:
/// an anchored result wins when it exists, otherwise the configured resolver
/// gets a chance to resolve the authored identifier.
#[derive(Clone)]
pub(super) struct SharedResolver(pub(super) Arc<dyn Resolver>);

impl Resolver for SharedResolver {
    fn create_identifier(&self, asset_path: &str, anchor: Option<&ResolvedPath>) -> String {
        let anchored = self.0.create_identifier(asset_path, anchor);
        if self.0.resolve(&anchored).is_some() {
            return anchored;
        }
        self.0
            .resolve(asset_path)
            .map(|resolved| resolved.to_string())
            .unwrap_or(anchored)
    }

    fn resolve(&self, asset_path: &str) -> Option<ResolvedPath> {
        self.0.resolve(asset_path)
    }

    fn resolve_for_new_asset(&self, asset_path: &str) -> Option<ResolvedPath> {
        self.0.resolve_for_new_asset(asset_path)
    }

    fn open_asset(
        &self,
        resolved_path: &ResolvedPath,
    ) -> std::io::Result<Box<dyn openusd::ar::Asset>> {
        self.0.open_asset(resolved_path)
    }

    fn identity(&self) -> String {
        self.0.identity()
    }
}

pub(super) fn resolve_asset_paths_with_resolver(
    resolver: &dyn Resolver,
    layer_path: &Path,
    authored: &str,
) -> Result<Vec<PathBuf>> {
    if authored.is_empty() {
        bail!("USD asset path is empty");
    }
    if authored.starts_with("anon:") {
        bail!("USD asset path is not a filesystem path: {authored}");
    }
    if authored.contains("<UDIM>") {
        return super::patterns::resolve_udim_pattern(layer_path, authored);
    }
    let anchor = ResolvedPath::new(layer_path.to_owned());
    let identifier = resolver.create_identifier(authored, Some(&anchor));
    let resolved = resolver
        .resolve(&identifier)
        .or_else(|| resolver.resolve(authored))
        .with_context(|| format!("OpenUSD resolver could not resolve {authored}"))?;
    Ok(vec![resolved_filesystem_path(&resolved)?])
}

fn resolved_filesystem_path(resolved: &ResolvedPath) -> Result<PathBuf> {
    let resolved_string = resolved.to_string();
    let filesystem_path = split_package_relative_path_outer(&resolved_string)
        .map(|(package, _)| PathBuf::from(package))
        .unwrap_or_else(|| resolved.as_ref().to_owned());
    regular_file(&filesystem_path)
}

pub(super) fn filesystem_identifier(identifier: &str, resolver: &dyn Resolver) -> Option<PathBuf> {
    if identifier.starts_with("anon:") {
        return None;
    }
    if is_package_relative_path(identifier) {
        let resolved = resolver.resolve(identifier)?;
        return resolved_filesystem_path(&resolved).ok();
    }
    Some(PathBuf::from(identifier))
}

fn regular_file(path: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("read USD dependency metadata {}", path.display()))?;
    if metadata.is_file() && !metadata.file_type().is_symlink() {
        return fs::canonicalize(path)
            .with_context(|| format!("canonicalize USD dependency {}", path.display()));
    }
    bail!(
        "USD dependency must be a regular non-symlink file: {}",
        path.display()
    )
}
