use std::fs;
use std::path::Path;

use anyhow::Result;
use openusd::usd::Stage;
use usd_bevy::{LiveRevision, LiveStage};
use usd_git::{GitRepository, Repository};
use usd_model::SemanticSnapshot;
use usd_semantic::SemanticConfig;

use super::*;
use crate::project::semantic_store::{SemanticStore, TursoSemanticStore};

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

fn run_git(directory: &Path, args: &[&str]) {
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

    async fn get_snapshot(&self, _id: &usd_model::SnapshotId) -> Result<Option<SemanticSnapshot>> {
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
