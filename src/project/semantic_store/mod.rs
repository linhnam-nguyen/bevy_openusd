//! Durable semantic snapshots and queries.

mod migration;
mod schema;
pub(crate) mod sync;
mod turso;

use std::{future::Future, path::Path};

use anyhow::{Context, Result, bail};
use openusd::usd::Stage;
use usd_git::GitRepository;
use usd_model::{EntityKey, EntitySnapshot, SemanticSnapshot, SnapshotId, SnapshotSource};
use usd_semantic::{SemanticConfig, SemanticExtractor};

pub(crate) use query::{
    DistinctFieldKeys, DistinctFieldKeysResult, GroupField, SemanticFilter, SemanticGroup,
    SemanticKey, SemanticQuery, SemanticQueryResult, SemanticQueryRow, SortField,
};

mod query;

#[cfg(test)]
#[path = "../../../tests/unit/project/semantic_store/test_support.rs"]
pub(crate) mod test_support;
pub(crate) use turso::TursoSemanticStore;

/// Storage contract for committed semantic snapshots.
#[allow(async_fn_in_trait)]
pub(crate) trait SemanticStore {
    async fn put_snapshot(&mut self, snapshot: &SemanticSnapshot) -> Result<()>;
    async fn get_snapshot(&self, id: &SnapshotId) -> Result<Option<SemanticSnapshot>>;
    async fn get_entity(
        &self,
        snapshot: &SnapshotId,
        key: &EntityKey,
    ) -> Result<Option<EntitySnapshot>>;
    async fn get_commit_snapshot(&self, git_oid: &str) -> Result<Option<SemanticSnapshot>>;
    async fn query(
        &self,
        snapshot: &SnapshotId,
        request: &SemanticQuery,
    ) -> Result<SemanticQueryResult>;
}

/// Load a cached commit snapshot, regenerating and persisting it on a miss.
pub(crate) async fn get_or_regenerate_commit_snapshot<F, Fut>(
    store: &mut impl SemanticStore,
    git_oid: &str,
    regenerate: F,
) -> Result<SemanticSnapshot>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<SemanticSnapshot>>,
{
    if let Some(snapshot) = store.get_commit_snapshot(git_oid).await? {
        return Ok(snapshot);
    }

    let snapshot = regenerate().await?;
    match &snapshot.source {
        SnapshotSource::GitCommit { oid } if oid == git_oid => {}
        SnapshotSource::GitCommit { oid } => {
            bail!("regenerated snapshot belongs to Git commit {oid}, expected {git_oid}")
        }
        SnapshotSource::Working { .. } => {
            bail!("regenerated snapshot is working state, expected committed snapshot")
        }
    }
    store.put_snapshot(&snapshot).await?;
    Ok(snapshot)
}

/// Materialize one Git revision and regenerate its complete semantic snapshot.
///
/// The caller supplies the relative path of the root USD stage because a Git
/// tree may contain more than one independent stage. Referenced layers remain
/// available below the materialized root.
pub(crate) fn regenerate_git_snapshot(
    repository_path: &Path,
    revision: &usd_git::RevisionId,
    destination: &Path,
    stage_relative_path: &Path,
    config: SemanticConfig,
) -> Result<SemanticSnapshot> {
    let repository = usd_git::Repository::open(repository_path).with_context(|| {
        format!(
            "opening USD Git repository at {}",
            repository_path.display()
        )
    })?;
    let materialized = repository
        .materialize_revision(revision, destination)
        .with_context(|| format!("materializing Git revision {revision}"))?;
    let stage_path = materialized.root.join(stage_relative_path);
    if !stage_path.is_file() {
        anyhow::bail!(
            "materialized USD stage does not exist: {}",
            stage_path.display()
        );
    }
    let stage_path_string = stage_path.to_string_lossy().into_owned();
    let stage = Stage::open(&stage_path_string)
        .with_context(|| format!("opening materialized USD stage {}", stage_path.display()))?;
    SemanticExtractor::new(config)
        .extract(
            &stage,
            SnapshotSource::GitCommit {
                oid: revision.to_string(),
            },
        )
        .with_context(|| format!("extracting semantic snapshot for Git revision {revision}"))
}

#[cfg(test)]
#[path = "../../../tests/unit/project/semantic_store/semantic_store_tests.rs"]
mod tests;
