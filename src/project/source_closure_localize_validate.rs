use std::{path::Path, sync::Arc};

use anyhow::{Context, Result, ensure};
use openusd::{
    ar::Resolver,
    usd::{InitialLoadSet, PrimPredicate},
};

pub(super) fn validate_localized_root(path: &Path, resolver: Arc<dyn Resolver>) -> Result<()> {
    let stage =
        super::super::discovery::open_stage_with_resolver(path, resolver, InitialLoadSet::LoadNone)
            .context("reopen localized USD root")?;
    stage
        .traverse(PrimPredicate::ALL, |_| {})
        .context("traverse localized USD root")?;
    ensure!(
        stage.composition_errors().is_empty(),
        "localized USD root has composition errors"
    );
    let default_prim = stage
        .default_prim()
        .context("localized USD root has no defaultPrim")?;
    let root_path = format!("/{default_prim}");
    ensure!(
        stage.prim(root_path.as_str()).is_defined()?,
        "localized USD root defaultPrim is not a defined prim"
    );
    Ok(())
}
