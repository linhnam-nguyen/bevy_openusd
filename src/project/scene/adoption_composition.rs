use std::{collections::HashSet, path::Path};

use anyhow::{Context, Result};
use openusd::usd::{PrimPredicate, Stage};

/// Detect relationships whose targets live in a different selected root.
/// Per-root references isolate those targets under `Root_i`; a root-layer
/// reference is required to preserve the authored cross-root topology.
pub(super) fn source_has_cross_root_relationship(
    source_metadata_path: &Path,
    source_prims: &[String],
) -> Result<bool> {
    let source_string = source_metadata_path
        .to_str()
        .context("Scene source metadata path must be valid UTF-8")?;
    let source_stage = Stage::open(source_string).context("inspect Scene source relationships")?;
    let selected_roots = source_prims
        .iter()
        .filter_map(|path| path.strip_prefix('/'))
        .filter_map(|path| path.split('/').next())
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    if selected_roots.is_empty() {
        return Ok(false);
    }
    let mut actual_roots = HashSet::new();
    for prim in source_stage.prim("/").children()? {
        if prim.is_active()? && prim.is_defined()? && !prim.is_abstract()? {
            if let Some(root) = top_level_root(prim.path().as_str()) {
                actual_roots.insert(root.to_owned());
            }
        }
    }
    let mut paths = Vec::new();
    source_stage.traverse(PrimPredicate::ALL, |path| paths.push(path.to_owned()))?;
    for path in paths {
        let Some(owner_root) = top_level_root(path.as_str()) else {
            continue;
        };
        if !selected_roots.contains(owner_root) {
            continue;
        }
        let prim = source_stage.prim(path.clone());
        let relationships = prim
            .relationships()
            .with_context(|| format!("inspect relationships on {}", path.as_str()))?;
        for relationship in relationships {
            let targets = relationship
                .targets()
                .with_context(|| format!("inspect relationship targets on {}", path.as_str()))?;
            if targets.into_iter().any(|target| {
                top_level_root(target.as_str()).is_some_and(|target_root| {
                    target_root != owner_root && actual_roots.contains(target_root)
                })
            }) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn top_level_root(path: &str) -> Option<&str> {
    path.strip_prefix('/')
        .and_then(|path| path.split('/').next())
}
