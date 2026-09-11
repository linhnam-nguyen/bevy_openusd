//! Bounded asynchronous prim-count inspection for uncached stages.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Mutex;

use anyhow::{Context, Result};
use bevy::prelude::Resource;
use openusd::usd::{PrimPredicate, Stage};
use usd_project::SceneId;

const REQUEST_CAPACITY: usize = 2;

#[derive(Debug)]
struct PrimCountRequest {
    freshness: PrimCountFreshness,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrimCountFreshness {
    pub(crate) scene_id: Option<SceneId>,
    pub(crate) cache_generation: Option<u64>,
    pub(crate) path: PathBuf,
    pub(crate) activation_generation: u64,
    pub(crate) session_id: u64,
}

#[derive(Debug)]
pub(crate) struct PrimCountResult {
    pub(crate) freshness: PrimCountFreshness,
    pub(crate) count: std::result::Result<usize, String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SubmitStatus {
    Queued,
    Full,
    Disconnected,
}

/// A single bounded worker for metadata-only prim counts.
///
/// The worker opens its own Stage from the source path. No Stage handle crosses
/// the thread boundary, and the result carries the path plus activation
/// generation so a late count cannot overwrite a newer activation.
#[derive(Resource, Debug)]
pub(crate) struct PrimCountWorker {
    requests: SyncSender<PrimCountRequest>,
    results: Mutex<Receiver<PrimCountResult>>,
}

impl PrimCountWorker {
    pub(crate) fn try_new() -> Result<Self> {
        let (request_sender, request_receiver) = mpsc::sync_channel(REQUEST_CAPACITY);
        let (result_sender, result_receiver) = mpsc::sync_channel(REQUEST_CAPACITY);
        std::thread::Builder::new()
            .name("usdview-prim-count".to_owned())
            .spawn(move || prim_count_worker(request_receiver, result_sender))
            .context("spawn prim-count worker")?;
        Ok(Self {
            requests: request_sender,
            results: Mutex::new(result_receiver),
        })
    }

    pub(crate) fn submit(&self, freshness: PrimCountFreshness) -> SubmitStatus {
        match self.requests.try_send(PrimCountRequest { freshness }) {
            Ok(()) => SubmitStatus::Queued,
            Err(TrySendError::Full(_)) => SubmitStatus::Full,
            Err(TrySendError::Disconnected(_)) => SubmitStatus::Disconnected,
        }
    }

    pub(crate) fn drain_results(&self) -> Vec<PrimCountResult> {
        self.results
            .lock()
            .map_or_else(|_| Vec::new(), |results| results.try_iter().collect())
    }
}

fn prim_count_worker(requests: Receiver<PrimCountRequest>, results: SyncSender<PrimCountResult>) {
    while let Ok(request) = requests.recv() {
        let count = count_stage_prims(&request.freshness.path).map_err(|error| format!("{error:#}"));
        if results
            .send(PrimCountResult {
                freshness: request.freshness,
                count,
            })
            .is_err()
        {
            break;
        }
    }
}

fn count_stage_prims(path: &std::path::Path) -> Result<usize> {
    let stage_path = path
        .to_str()
        .context("prim-count source path must be valid UTF-8")?;
    let stage = Stage::open(stage_path).context("open stage for asynchronous prim count")?;
    let mut count = 0usize;
    stage
        .traverse(PrimPredicate::DEFAULT, |_| count = count.saturating_add(1))
        .context("traverse stage for asynchronous prim count")?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::PrimCountFreshness;

    #[test]
    fn freshness_token_includes_scene_cache_and_runtime_identity() {
        let base = PrimCountFreshness {
            scene_id: None,
            cache_generation: None,
            path: "/tmp/scene.usda".into(),
            activation_generation: 4,
            session_id: 9,
        };
        let mut changed = base.clone();
        changed.activation_generation += 1;
        assert_ne!(base, changed);
        changed = base.clone();
        changed.path = "/tmp/other.usda".into();
        assert_ne!(base, changed);
    }
}
