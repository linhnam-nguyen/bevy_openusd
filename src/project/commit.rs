//! Runtime-memory to Git commit coordination.

use std::path::Path;

use anyhow::{Context, Result, bail};
use openusd::usd::{PrimPredicate, Stage};
use usd_bevy::{LiveRevision, LiveStage};
use usd_git::{CommitRequest, GitRepository, Repository, RevisionId};
use usd_model::{SemanticSnapshot, SnapshotSource};
use usd_semantic::{SemanticConfig, SemanticExtractor};

use super::semantic_store::SemanticStore;

/// Commit/base state owned by the application project layer.
#[derive(Debug, Default)]
pub(crate) struct CommitState {
    base_revision: Option<RevisionId>,
    dirty: bool,
    committing: bool,
}

impl CommitState {
    pub(crate) fn new(base_revision: Option<RevisionId>) -> Self {
        Self {
            base_revision,
            ..Self::default()
        }
    }

    pub(crate) fn base_revision(&self) -> Option<&RevisionId> {
        self.base_revision.as_ref()
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) fn is_committing(&self) -> bool {
        self.committing
    }

    pub(crate) fn mark_dirty(&mut self) -> Result<()> {
        if self.committing {
            bail!("a commit is already in progress; write transaction rejected")
        }
        self.dirty = true;
        Ok(())
    }
}

/// Result of a successful Git commit.
///
/// `semantic_persisted` can be false even though the Git commit succeeded:
/// the semantic snapshot is a rebuildable cache and will be regenerated later.
#[derive(Debug)]
pub(crate) struct CommitOutcome {
    pub(crate) revision: RevisionId,
    pub(crate) snapshot: Option<SemanticSnapshot>,
    pub(crate) semantic_persisted: bool,
    pub(crate) semantic_error: Option<String>,
}

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
    if state.committing {
        bail!("a commit is already in progress")
    }
    if !state.dirty {
        bail!("cannot commit a clean runtime state")
    }
    validate_relative_stage_path(stage_relative_path)?;
    if live_stage.current_revision() != expected_live_revision {
        bail!(
            "live stage revision changed before commit: expected {}, found {}",
            expected_live_revision.0,
            live_stage.current_revision().0
        );
    }
    state.committing = true;

    let result = commit_live_stage_inner(
        state,
        live_stage,
        repository_path,
        stage_relative_path,
        message,
        config,
        semantic_store,
    )
    .await;
    state.committing = false;
    result
}

async fn commit_live_stage_inner(
    state: &mut CommitState,
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

    let revision = repository
        .create_commit(CommitRequest::new(message, staging.path()))
        .context("creating Git commit from staged USD tree")?;

    state.base_revision = Some(revision.clone());
    state.dirty = false;

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

fn validate_relative_stage_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("commit stage path must be a non-empty relative path")
    }
    if path
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        bail!("commit stage path must not contain parent traversal")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;
    use openusd::usd::Stage;
    use usd_git::{GitRepository, Repository};

    use super::*;
    use crate::project::semantic_store::TursoSemanticStore;

    fn create_repository() -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("create repository directory");
        run_git(directory.path(), &["init", "-b", "main"]);
        run_git(directory.path(), &["config", "user.name", "USDHub Tests"]);
        run_git(
            directory.path(),
            &["config", "user.email", "tests@usdhub.invalid"],
        );
        fs::write(
            directory.path().join("model.usda"),
            b"#usda 1.0\ndef Xform \"World\" {}\n",
        )
        .unwrap();
        run_git(directory.path(), &["add", "."]);
        run_git(directory.path(), &["commit", "-m", "initial"]);
        directory
    }

    fn run_git(directory: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(directory)
            .args(args)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn commit_failure_keeps_runtime_dirty_and_head_unchanged() {
        let repository_dir = create_repository();
        let repository = Repository::open(repository_dir.path()).unwrap();
        let before = repository.head().unwrap().unwrap();
        let live_stage = LiveStage::new(
            Stage::open(repository_dir.path().join("model.usda").to_str().unwrap()).unwrap(),
        );
        let mut state = CommitState::new(Some(before.id().clone()));
        state.mark_dirty().unwrap();
        let mut store = runtime()
            .block_on(TursoSemanticStore::open_memory())
            .unwrap();

        let result = runtime().block_on(commit_live_stage(
            &mut state,
            &live_stage,
            LiveRevision::default(),
            repository_dir.path(),
            Path::new("../unsafe/model.usda"),
            "rejected",
            SemanticConfig::default(),
            &mut store,
        ));
        assert!(result.is_err());
        assert!(state.is_dirty());
        assert!(!state.is_committing());
        assert_eq!(repository.head().unwrap().unwrap(), before);
    }

    #[test]
    fn stale_live_revision_is_rejected_before_git_changes() {
        let repository_dir = create_repository();
        let repository = Repository::open(repository_dir.path()).unwrap();
        let before = repository.head().unwrap().unwrap();
        let live_stage = LiveStage::new(
            Stage::open(repository_dir.path().join("model.usda").to_str().unwrap()).unwrap(),
        );
        let mut state = CommitState::new(Some(before.id().clone()));
        state.mark_dirty().unwrap();
        let mut store = runtime()
            .block_on(TursoSemanticStore::open_memory())
            .unwrap();

        let result = runtime().block_on(commit_live_stage(
            &mut state,
            &live_stage,
            LiveRevision(1),
            repository_dir.path(),
            Path::new("model.usda"),
            "stale revision",
            SemanticConfig::default(),
            &mut store,
        ));
        assert!(result.is_err());
        assert!(state.is_dirty());
        assert!(!state.is_committing());
        assert_eq!(repository.head().unwrap().unwrap(), before);
    }

    #[test]
    fn successful_commit_updates_base_and_persists_semantics() -> Result<()> {
        let repository_dir = create_repository();
        let repository = Repository::open(repository_dir.path())?;
        let before = repository.head()?.unwrap();
        let live_stage = LiveStage::new(Stage::open(
            repository_dir.path().join("model.usda").to_str().unwrap(),
        )?);
        usd_bevy::authoring::define_prim(&live_stage.stage, "/CommittedEdit", "Xform")?;
        let batch = live_stage
            .drain_change_batch()
            .expect("live authoring should produce a change batch");
        let mut state = CommitState::new(Some(before.id().clone()));
        state.mark_dirty()?;
        let mut store = runtime().block_on(TursoSemanticStore::open_memory())?;

        let outcome = runtime().block_on(commit_live_stage(
            &mut state,
            &live_stage,
            batch.revision,
            repository_dir.path(),
            Path::new("model.usda"),
            "commit live stage",
            SemanticConfig::default(),
            &mut store,
        ))?;
        assert_ne!(outcome.revision, *before.id());
        assert!(outcome.semantic_persisted);
        assert!(outcome.semantic_error.is_none());
        assert!(outcome.snapshot.is_some());
        assert!(!state.is_dirty());
        assert_eq!(state.base_revision(), Some(&outcome.revision));
        assert_eq!(repository.head()?.unwrap().id(), &outcome.revision);
        let materialized = tempfile::tempdir()?;
        repository.materialize_revision(&outcome.revision, materialized.path())?;
        let committed = fs::read_to_string(materialized.path().join("model.usda"))?;
        assert!(committed.contains("CommittedEdit"));
        Ok(())
    }

    #[test]
    fn semantic_cache_failure_does_not_invalidate_git_commit() -> Result<()> {
        let repository_dir = create_repository();
        let repository = Repository::open(repository_dir.path())?;
        let before = repository.head()?.unwrap();
        let live_stage = LiveStage::new(Stage::open(
            repository_dir.path().join("model.usda").to_str().unwrap(),
        )?);
        let mut state = CommitState::new(Some(before.id().clone()));
        state.mark_dirty()?;
        let mut store = FailingStore;

        let outcome = runtime().block_on(commit_live_stage(
            &mut state,
            &live_stage,
            LiveRevision::default(),
            repository_dir.path(),
            Path::new("model.usda"),
            "cache may rebuild",
            SemanticConfig::default(),
            &mut store,
        ))?;
        assert_ne!(outcome.revision, *before.id());
        assert!(!outcome.semantic_persisted);
        assert!(outcome.semantic_error.is_some());
        assert_eq!(repository.head()?.unwrap().id(), &outcome.revision);
        assert!(!state.is_dirty());
        Ok(())
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    struct FailingStore;

    impl SemanticStore for FailingStore {
        async fn put_snapshot(&mut self, _snapshot: &SemanticSnapshot) -> Result<()> {
            Err(anyhow::anyhow!("intentional cache failure"))
        }

        async fn get_snapshot(
            &self,
            _id: &usd_model::SnapshotId,
        ) -> Result<Option<SemanticSnapshot>> {
            Ok(None)
        }

        async fn get_entity(
            &self,
            _snapshot: &usd_model::SnapshotId,
            _key: &usd_model::EntityKey,
        ) -> Result<Option<usd_model::EntitySnapshot>> {
            Ok(None)
        }

        async fn get_commit_snapshot(&self, _git_oid: &str) -> Result<Option<SemanticSnapshot>> {
            Ok(None)
        }

        async fn query(
            &self,
            _snapshot: &usd_model::SnapshotId,
            _request: &crate::project::semantic_store::SemanticQuery,
        ) -> Result<crate::project::semantic_store::SemanticQueryResult> {
            Ok(Default::default())
        }
    }
}
