use anyhow::{Context, Result, bail};
use openusd::usd::{PrimPredicate, Stage};
use std::path::Path;
use usd_bevy::{LiveRevision, LiveStage};
use usd_git::{CommitRequest, GitRepository, Repository};
use usd_model::SnapshotSource;
use usd_semantic::{SemanticConfig, SemanticExtractor};

use super::super::semantic_store::SemanticStore;
use super::state::{CommitOutcome, CommitState, validate_relative_stage_path};

/// Commit the current live stage as one canonical Git revision.
///
/// Errors returned before Git creation leave `CommitState::dirty` unchanged and
/// leave `HEAD` untouched. Once Git succeeds, the new revision is authoritative
/// even if semantic persistence reports an error.
pub(crate) async fn commit_live_stage(
    state: &mut CommitState,
    live_stage: &LiveStage,
    expected_live_revision: LiveRevision,
    repository_path: &Path,
    stage_relative_path: &Path,
    message: &str,
    config: SemanticConfig,
    semantic_store: &mut impl SemanticStore,
) -> Result<CommitOutcome> {
    validate_relative_stage_path(stage_relative_path)?;
    if live_stage.current_revision() != expected_live_revision {
        bail!(
            "live stage revision changed before commit: expected {}, found {}",
            expected_live_revision.0,
            live_stage.current_revision().0
        );
    }
    let lease = state.acquire_lease(expected_live_revision)?;

    let result = commit_live_stage_inner(
        state,
        lease,
        live_stage,
        repository_path,
        stage_relative_path,
        message,
        config,
        semantic_store,
    )
    .await;
    state.release_lease();
    result
}

async fn commit_live_stage_inner(
    state: &mut CommitState,
    mut lease: super::state::CommitLease,
    live_stage: &LiveStage,
    repository_path: &Path,
    stage_relative_path: &Path,
    message: &str,
    config: SemanticConfig,
    semantic_store: &mut impl SemanticStore,
) -> Result<CommitOutcome> {
    let mut repository = Repository::open(repository_path)
        .with_context(|| format!("opening Git repository {}", repository_path.display()))?;
    let head = repository
        .head()
        .context("reading Git HEAD before commit")?
        .context("cannot commit without an existing Git HEAD")?;
    if state
        .base_revision
        .as_ref()
        .is_some_and(|base| base != head.id())
    {
        bail!("Git HEAD changed since the runtime base revision was captured")
    }

    let staging = tempfile::tempdir().context("creating temporary commit staging directory")?;
    repository
        .materialize_revision(head.id(), staging.path())
        .with_context(|| format!("materializing Git base revision {}", head.id()))?;
    let stage_path = staging.path().join(stage_relative_path);
    if let Some(parent) = stage_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("creating staged USD parent directory {}", parent.display())
        })?;
    }
    let stage_path_string = stage_path.to_string_lossy().into_owned();
    live_stage
        .stage
        .root_layer()
        .export(&stage_path_string)
        .with_context(|| format!("exporting live USD stage to {}", stage_path.display()))?;

    let validated_stage = Stage::open(&stage_path_string)
        .with_context(|| format!("reopening staged USD file {}", stage_path.display()))?;
    validated_stage
        .traverse(PrimPredicate::DEFAULT, |_| {})
        .context("validating staged USD composition")?;

    if live_stage.current_revision() != lease.expected_live_revision() {
        bail!(
            "live stage revision changed while commit was staged: expected {}, found {}",
            lease.expected_live_revision().0,
            live_stage.current_revision().0
        );
    }

    let revision = repository
        .create_commit(CommitRequest::new(message, staging.path()))
        .context("creating Git commit from staged USD tree")?;

    lease.finalize_git_commit(state, revision.clone());

    let snapshot_result = SemanticExtractor::new(config).extract(
        &validated_stage,
        SnapshotSource::GitCommit {
            oid: revision.to_string(),
        },
    );
    let snapshot = match snapshot_result {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            return Ok(CommitOutcome {
                revision,
                snapshot: None,
                semantic_persisted: false,
                semantic_error: Some(format!("semantic extraction failed: {error:#}")),
            });
        }
    };

    let mut semantic_error = None;
    let semantic_persisted = match semantic_store
        .put_snapshot(snapshot.as_ref().unwrap())
        .await
    {
        Ok(()) => true,
        Err(error) => {
            semantic_error = Some(format!("semantic snapshot persistence failed: {error:#}"));
            false
        }
    };

    Ok(CommitOutcome {
        revision,
        snapshot,
        semantic_persisted,
        semantic_error,
    })
}
