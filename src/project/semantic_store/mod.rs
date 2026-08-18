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
    GroupField, SemanticFilter, SemanticGroup, SemanticQuery, SemanticQueryResult,
    SemanticQueryRow, SortField, SortRule,
};

mod query;

pub(crate) use schema::SCHEMA_VERSION;
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
mod tests {
    use std::{fs, path::Path, process::Command};

    use usd_git::GitRepository;

    use super::regenerate_git_snapshot;

    fn git(repository: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(repository)
            .args(args)
            .output()
            .expect("git should be installed for the regeneration test");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn materialized_git_revision_regenerates_a_snapshot() {
        let repository_dir = tempfile::tempdir().expect("repository tempdir should create");
        git(repository_dir.path(), &["init"]);
        git(
            repository_dir.path(),
            &["config", "user.name", "USDHub Tests"],
        );
        git(
            repository_dir.path(),
            &["config", "user.email", "tests@usdhub.invalid"],
        );
        fs::write(
            repository_dir.path().join("model.usda"),
            include_bytes!("../../../crates/usd_semantic/tests/fixtures/identity_original.usda"),
        )
        .expect("USD fixture should write");
        git(repository_dir.path(), &["add", "model.usda"]);
        git(repository_dir.path(), &["commit", "-m", "initial"]);

        let repository =
            usd_git::Repository::open(repository_dir.path()).expect("repository opens");
        let revision = repository
            .head()
            .expect("HEAD resolves")
            .expect("HEAD exists");
        let destination = tempfile::tempdir().expect("materialization tempdir should create");
        let snapshot = regenerate_git_snapshot(
            repository_dir.path(),
            revision.id(),
            destination.path().join("tree").as_path(),
            Path::new("model.usda"),
            usd_semantic::SemanticConfig::default(),
        )
        .expect("materialized stage should regenerate");

        assert_eq!(
            snapshot.source,
            usd_model::SnapshotSource::GitCommit {
                oid: revision.id().to_string()
            }
        );
        assert!(!snapshot.entities.is_empty());
    }
}
