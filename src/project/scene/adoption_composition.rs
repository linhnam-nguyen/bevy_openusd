use std::path::Path;

use anyhow::{Context, Result};
use openusd::usd::{PrimPredicate, Stage};

/// Detect relationships whose targets live in a different selected root.
/// Per-root references isolate those targets under `Root_i`; a root-layer
/// reference is required to preserve the authored cross-root topology.
pub(super) fn source_has_cross_root_relationship(
    source_metadata_path: &Path,
    source_prims: &[String],
) -> Result<bool> {
    if source_prims.len() < 2 {
        return Ok(false);
    }
    let source_string = source_metadata_path
        .to_str()
        .context("Scene source metadata path must be valid UTF-8")?;
    let source_stage = Stage::open(source_string).context("inspect Scene source relationships")?;
    let root_names = source_prims
        .iter()
        .filter_map(|path| path.strip_prefix('/'))
        .filter_map(|path| path.split('/').next())
        .collect::<std::collections::HashSet<_>>();
    let mut found = false;
    source_stage.traverse(PrimPredicate::ALL, |path| {
        if found {
            return;
        }
        let Some(owner_root) = path
            .as_str()
            .strip_prefix('/')
            .and_then(|path| path.split('/').next())
        else {
            return;
        };
        let prim = source_stage.prim(path.clone());
        let Ok(relationships) = prim.relationships() else {
            return;
        };
        found = relationships.into_iter().any(|relationship| {
            relationship
                .targets()
                .map(|targets| {
                    targets.into_iter().any(|target| {
                        target
                            .as_str()
                            .strip_prefix('/')
                            .and_then(|path| path.split('/').next())
                            .is_some_and(|target_root| {
                                target_root != owner_root && root_names.contains(target_root)
                            })
                    })
                })
                .unwrap_or(false)
        });
    })?;
    Ok(found)
}
