//! Bounded, off-ECS-thread semantic property history.
//!
//! The Bevy bridge only admits the newest request. This worker owns repository
//! discovery, history traversal, USD materialization, and semantic extraction;
//! no Git or filesystem history work runs in `Update`.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use bevy::prelude::Resource;
use openusd::usd::Stage;
use usd_git::{CommitInfo, GitRepository, Repository, RevisionId};
use usd_model::{CanonicalValue, EntityKey, SemanticSnapshot, SnapshotSource};
use usd_semantic::{SemanticConfig, SemanticExtractor};
use viewport_protocol::{BimPropertyProvenanceReadModel, BimPropertyProvenanceStatus, SceneAnchor};

use super::scene_query::LatestMailbox;

const MAX_PROVENANCE_HISTORY: usize = 64;

#[derive(Debug)]
struct BimProvenanceJob {
    request_id: String,
    target: SceneAnchor,
    property: String,
    entity_key: EntityKey,
    history_head: RevisionId,
    stage_path: PathBuf,
    activation_generation: u64,
    generation: u64,
}

#[derive(Debug)]
pub(crate) struct BimProvenanceResult {
    pub(crate) request_id: String,
    pub(crate) activation_generation: u64,
    pub(crate) result: Result<BimPropertyProvenanceReadModel, String>,
}

#[derive(Debug, Resource)]
pub(crate) struct BimProvenanceService {
    jobs: Arc<LatestMailbox<BimProvenanceJob>>,
    results: Arc<LatestMailbox<BimProvenanceResult>>,
    generation: Arc<AtomicU64>,
}

impl Default for BimProvenanceService {
    fn default() -> Self {
        let jobs = Arc::new(LatestMailbox::new());
        let results = Arc::new(LatestMailbox::new());
        let generation = Arc::new(AtomicU64::new(0));
        let worker_jobs = Arc::clone(&jobs);
        let worker_results = Arc::clone(&results);
        let worker_generation = Arc::clone(&generation);

        std::thread::Builder::new()
            .name("usdview-bim-provenance".to_owned())
            .spawn(move || provenance_worker(worker_jobs, worker_results, worker_generation))
            .expect("BIM provenance worker should start");

        Self {
            jobs,
            results,
            generation,
        }
    }
}

impl BimProvenanceService {
    pub(crate) fn submit(
        &self,
        request_id: String,
        target: SceneAnchor,
        property: String,
        entity_key: EntityKey,
        history_head: RevisionId,
        stage_path: PathBuf,
        activation_generation: u64,
    ) -> bool {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.jobs
            .replace(BimProvenanceJob {
                request_id,
                target,
                property,
                entity_key,
                history_head,
                stage_path,
                activation_generation,
                generation,
            })
            .is_ok()
    }

    pub(crate) fn drain_results(&self) -> Vec<BimProvenanceResult> {
        self.results.take().into_iter().collect()
    }
}

impl Drop for BimProvenanceService {
    fn drop(&mut self) {
        self.jobs.close();
        self.results.close();
    }
}

fn provenance_worker(
    jobs: Arc<LatestMailbox<BimProvenanceJob>>,
    results: Arc<LatestMailbox<BimProvenanceResult>>,
    generation: Arc<AtomicU64>,
) {
    while let Some(job) = jobs.pop() {
        if generation.load(Ordering::Acquire) != job.generation {
            continue;
        }
        let request_id = job.request_id.clone();
        let result = resolve_job(&job, || {
            generation.load(Ordering::Acquire) == job.generation
        });
        if !is_current(&job, &generation) {
            continue;
        }
        let result = match result {
            ResolveOutcome::Cancelled => continue,
            ResolveOutcome::Completed(result) => result,
        };
        if results
            .replace(BimProvenanceResult {
                request_id,
                activation_generation: job.activation_generation,
                result,
            })
            .is_err()
        {
            break;
        }
    }
}

fn is_current(job: &BimProvenanceJob, generation: &AtomicU64) -> bool {
    generation.load(Ordering::Acquire) == job.generation
}

enum ResolveOutcome {
    Cancelled,
    Completed(Result<BimPropertyProvenanceReadModel, String>),
}

fn resolve_job<F>(job: &BimProvenanceJob, is_current: F) -> ResolveOutcome
where
    F: Fn() -> bool,
{
    let result: Result<BimPropertyProvenanceReadModel, ResolveError> = (|| {
        let repository_path = job.stage_path.parent().unwrap_or_else(|| Path::new("."));
        let repository = Repository::open(repository_path).map_err(|error| {
            ResolveError::Failed(format!("opening BIM provenance repository: {error}"))
        })?;
        let Some(repository_root) = repository.work_dir() else {
            return Err(ResolveError::Failed(
                "BIM provenance requires a repository working tree".to_owned(),
            ));
        };
        let repository_root = fs::canonicalize(repository_root).map_err(|error| {
            ResolveError::Failed(format!("resolving BIM provenance repository root: {error}"))
        })?;
        let stage_path = fs::canonicalize(&job.stage_path).map_err(|error| {
            ResolveError::Failed(format!("resolving BIM provenance stage path: {error}"))
        })?;
        let stage_relative_path = stage_path
            .strip_prefix(&repository_root)
            .map_err(|_| {
                ResolveError::Failed(
                    "BIM provenance stage is outside its Git repository".to_owned(),
                )
            })?
            .to_owned();
        if stage_relative_path.as_os_str().is_empty() {
            return Err(ResolveError::Failed(
                "BIM provenance stage path is empty".to_owned(),
            ));
        }

        let history = repository
            .history(&job.history_head, MAX_PROVENANCE_HISTORY)
            .map_err(|error| {
                ResolveError::Failed(format!("reading bounded BIM provenance history: {error}"))
            })?;
        let mut values = HashMap::with_capacity(history.len().saturating_mul(2));
        let change = find_last_property_change(&history, |revision| {
            if !is_current() {
                return Err(HistoryLookupError::Cancelled);
            }
            let value = property_value_for_commit(
                &repository,
                revision,
                &stage_relative_path,
                &job.entity_key,
                &job.property,
                &mut values,
            )
            .map_err(HistoryLookupError::Failed)?;
            if !is_current() {
                return Err(HistoryLookupError::Cancelled);
            }
            Ok(value)
        });
        let change = match change {
            Ok(change) => change,
            Err(HistoryLookupError::Cancelled) => return Err(ResolveError::Cancelled),
            Err(HistoryLookupError::Failed(error)) => return Err(ResolveError::Failed(error)),
        };
        let Some((commit, old_value, new_value)) = change else {
            return Ok(unavailable(&job.target, &job.property, &job.history_head));
        };
        Ok(BimPropertyProvenanceReadModel {
            target: job.target.clone(),
            property: job.property.clone(),
            history_head: job.history_head.to_string(),
            status: BimPropertyProvenanceStatus::Available,
            commit_id: Some(commit.id.to_string()),
            commit_message: Some(commit.message),
            author_name: Some(commit.author.name),
            author_email: Some(commit.author.email),
            authored_at_seconds: Some(commit.author.time_seconds),
            old_value,
            new_value,
        })
    })();

    match result {
        Ok(result) => ResolveOutcome::Completed(Ok(result)),
        Err(ResolveError::Cancelled) => ResolveOutcome::Cancelled,
        Err(ResolveError::Failed(error)) => ResolveOutcome::Completed(Err(error)),
    }
}

enum ResolveError {
    Cancelled,
    Failed(String),
}

#[derive(Debug)]
enum HistoryLookupError {
    Cancelled,
    Failed(String),
}

fn property_value_for_commit(
    repository: &Repository,
    revision: &RevisionId,
    stage_relative_path: &Path,
    entity_key: &EntityKey,
    property: &str,
    values: &mut HashMap<String, Option<CanonicalValue>>,
) -> Result<Option<CanonicalValue>, String> {
    if let Some(value) = values.get(revision.as_str()) {
        return Ok(value.clone());
    }

    let materialized = tempfile::tempdir()
        .map_err(|error| format!("creating BIM provenance materialization directory: {error}"))?;
    repository
        .materialize_revision(revision, materialized.path())
        .map_err(|error| format!("materializing BIM provenance commit {revision}: {error}"))?;
    let stage_path = materialized.path().join(stage_relative_path);
    let stage_path_string = stage_path.to_string_lossy().into_owned();
    let stage = Stage::open(&stage_path_string)
        .map_err(|error| format!("opening BIM provenance commit stage: {error}"))?;
    let snapshot = SemanticExtractor::new(SemanticConfig::default())
        .extract(
            &stage,
            SnapshotSource::GitCommit {
                oid: revision.to_string(),
            },
        )
        .map_err(|error| format!("extracting BIM provenance semantic snapshot: {error:#}"))?;
    let value = semantic_property_value(&snapshot, entity_key, property);
    values.insert(revision.to_string(), value.clone());
    Ok(value)
}

fn semantic_property_value(
    snapshot: &SemanticSnapshot,
    entity_key: &EntityKey,
    property: &str,
) -> Option<CanonicalValue> {
    snapshot
        .entities
        .get(entity_key)?
        .properties
        .iter()
        .find(|candidate| candidate.name == property)
        .map(|candidate| candidate.value.clone())
}

fn unavailable(
    target: &SceneAnchor,
    property: &str,
    history_head: &RevisionId,
) -> BimPropertyProvenanceReadModel {
    BimPropertyProvenanceReadModel {
        target: target.clone(),
        property: property.to_owned(),
        history_head: history_head.to_string(),
        status: BimPropertyProvenanceStatus::Unavailable,
        commit_id: None,
        commit_message: None,
        author_name: None,
        author_email: None,
        authored_at_seconds: None,
        old_value: None,
        new_value: None,
    }
}

fn find_last_property_change<F>(
    history: &[CommitInfo],
    mut property_value: F,
) -> Result<Option<(CommitInfo, Option<CanonicalValue>, Option<CanonicalValue>)>, HistoryLookupError>
where
    F: FnMut(&RevisionId) -> Result<Option<CanonicalValue>, HistoryLookupError>,
{
    for commit in history {
        let new_value = property_value(&commit.id)?;
        if commit.parents.is_empty() {
            if new_value.is_some() {
                return Ok(Some((commit.clone(), None, new_value)));
            }
            continue;
        }
        for parent in &commit.parents {
            let old_value = property_value(parent)?;
            if old_value != new_value {
                return Ok(Some((commit.clone(), old_value, new_value.clone())));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "bim_provenance_tests.rs"]
mod tests;
