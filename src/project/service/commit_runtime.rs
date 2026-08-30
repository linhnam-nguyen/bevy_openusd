//! Runtime-owned Project commit staging and derived semantic persistence.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use openusd::usd::{PrimPredicate, Stage};
use project_protocol::{ProjectCommitTarget, ProjectWriteError, ProjectWriteErrorCode};
use usd_model::SnapshotSource;
use usd_project::{ProjectId, SceneId};
use usd_semantic::{SemanticConfig, SemanticExtractor};

use super::{ProjectRuntimeAuthority, ProjectRuntimeSnapshot};
use crate::project::semantic_store::{SemanticStore, TursoSemanticStore};

const SCENES_RELATIVE_DIRECTORY: &str = ".usdhub/scenes";
const SEMANTIC_CACHE_RELATIVE_PATH: &str = ".usdhub/cache/semantic-snapshots.db";

/// Keep a runtime lease alive until the Project commit either succeeds or
/// unwinds. The render owner can therefore release its lease on failure.
pub(super) struct RuntimeLeaseGuard {
    authority: Arc<dyn ProjectRuntimeAuthority>,
    project_root: PathBuf,
    project_id: ProjectId,
    lease_id: Option<String>,
}

impl RuntimeLeaseGuard {
    pub(super) fn new(
        authority: Arc<dyn ProjectRuntimeAuthority>,
        project_root: PathBuf,
        project_id: ProjectId,
        snapshot: Option<&ProjectRuntimeSnapshot>,
    ) -> Self {
        Self {
            authority,
            project_root,
            project_id,
            lease_id: snapshot.map(|snapshot| snapshot.lease_id.clone()),
        }
    }

    pub(super) fn clear(&mut self) {
        self.lease_id = None;
    }
}

impl Drop for RuntimeLeaseGuard {
    fn drop(&mut self) {
        if let Some(lease_id) = self.lease_id.take() {
            self.authority
                .abort_commit(&self.project_root, self.project_id, &lease_id);
        }
    }
}

pub(super) fn overlay_runtime_snapshot(
    project_root: &Path,
    staging: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    target: &ProjectCommitTarget,
    snapshot: &ProjectRuntimeSnapshot,
) -> Result<(), ProjectWriteError> {
    let allowed = match target {
        ProjectCommitTarget::Project => manifest.scenes().iter().map(|scene| scene.id).collect(),
        ProjectCommitTarget::Scene(scene_id) => {
            super::scene_closure::scene_commit_closure(project_root, manifest.raw(), *scene_id)
                .map_err(|_| ProjectWriteError::Failed {
                    code: ProjectWriteErrorCode::ConcurrentChange,
                })?
                .0
        }
    };
    if !allowed.contains(&snapshot.scene_id) {
        return Err(ProjectWriteError::Invalid {
            code: ProjectWriteErrorCode::SceneNotFound,
        });
    }
    let path = staging
        .join(SCENES_RELATIVE_DIRECTORY)
        .join(format!("{}.usda", snapshot.scene_id));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| commit_error())?;
    }
    fs::write(&path, &snapshot.root_layer).map_err(|_| commit_error())?;
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string).map_err(|_| commit_error())?;
    stage
        .traverse(PrimPredicate::DEFAULT, |_| {})
        .map_err(|_| commit_error())?;
    Ok(())
}

/// Persist the exact staged runtime revision as a rebuildable semantic cache.
/// Git remains authoritative: callers log this failure after Git succeeds.
pub(super) fn persist_semantic_snapshot(
    project_root: &Path,
    staging: &Path,
    scene_id: SceneId,
    revision: &str,
) -> Result<()> {
    let stage_path = staging
        .join(SCENES_RELATIVE_DIRECTORY)
        .join(format!("{scene_id}.usda"));
    let stage_path_string = stage_path.to_string_lossy().into_owned();
    let stage = Stage::open(&stage_path_string)
        .with_context(|| format!("open committed runtime Scene {}", stage_path.display()))?;
    stage
        .traverse(PrimPredicate::DEFAULT, |_| {})
        .context("validate committed runtime Scene before semantic extraction")?;
    let snapshot = SemanticExtractor::new(SemanticConfig::default()).extract(
        &stage,
        SnapshotSource::GitCommit {
            oid: revision.to_owned(),
        },
    )?;
    let database_path = project_root.join(SEMANTIC_CACHE_RELATIVE_PATH);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create semantic persistence runtime")?;
    let mut store = runtime.block_on(TursoSemanticStore::open(database_path))?;
    runtime.block_on(store.put_snapshot(&snapshot))
}

fn commit_error() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::CommitFailed,
    }
}
