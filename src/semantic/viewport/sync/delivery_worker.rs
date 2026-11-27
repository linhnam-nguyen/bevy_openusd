use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::Instant;

use bevy::prelude::Resource;
use usd_bevy::LiveRevision;
use usd_model::SemanticSnapshot;
use viewport_protocol::RuntimeProfile;

use crate::cache::blobs::{
    BlobStore, FilesystemBlobStore, OBJECTS_DIRECTORY, PreparedMeshBlob,
};
use crate::cache::hydration::ActiveProjectCacheContext;
use crate::document::runtime::delivery::{
    RuntimeDeliveryBundle, build_runtime_delivery_with_payloads,
};
use crate::document::runtime::payload::{PreparedRuntimeBlob, PreparedRuntimePayloads};

pub(crate) const DELIVERY_QUEUE_CAPACITY: usize = 4;
pub(crate) const DELIVERY_RESULT_CAPACITY: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeDeliveryIdentity {
    pub(crate) session_id: u64,
    pub(crate) live_revision: LiveRevision,
    pub(crate) projection_generation: u64,
}

/// Sealed canonical source provenance captured before delivery extraction.
///
/// Binds an asynchronous `DeliveryWork` payload to the exact immutable source
/// revision and authorized desired generation observed at capture time so
/// `build_delivery` never stamps a payload with a post-extraction disk read or
/// supersedes a newer durable generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DeliveryCacheProvenance {
    pub(crate) generation: u64,
    pub(crate) revision_digest: usd_model::HashDigest,
}

impl DeliveryCacheProvenance {
    /// Capture sealed canonical provenance for a clean (unmodified) live stage.
    /// Returns `None` if the stage carries unsaved in-memory edits, if the
    /// target is retired/superseded, or if canonical revision qualification
    /// fails — allowing runtime delivery to proceed without persistent cache
    /// publication.
    pub(crate) fn capture(
        context: &ActiveProjectCacheContext,
        live: Option<&usd_bevy::LiveStage>,
    ) -> Option<Self> {
        if live.is_some_and(|stage| {
            stage.authoring_generation() != LiveRevision::default()
                || stage.current_revision() != LiveRevision::default()
        }) {
            bevy::log::debug!(
                "[project-cache] live stage has unsaved edits; persistent delivery publication declined"
            );
            return None;
        }
        let revision_digest = match context
            .identity
            .revision_digest_from_snapshot(&context.project_root, live)
        {
            Ok(digest) => digest,
            Err(error) => {
                bevy::log::debug!(
                    "[project-cache] delivery provenance could not be qualified: {error:#}"
                );
                return None;
            }
        };
        let store = crate::cache::ProjectCacheStore::new(&context.project_root);
        let generation = match store.ensure_matching_generation(&context.identity, revision_digest) {
            Ok(Some(generation)) => generation,
            Ok(None) => return None,
            Err(error) => {
                bevy::log::debug!(
                    "[project-cache] delivery generation could not be authorized: {error:#}"
                );
                return None;
            }
        };
        Some(Self {
            generation,
            revision_digest,
        })
    }
}

#[derive(Debug)]
pub(crate) struct PendingRuntimeDelivery {
    pub(crate) identity: RuntimeDeliveryIdentity,
    pub(crate) snapshot: Arc<SemanticSnapshot>,
    pub(crate) prepared_blobs: Vec<PreparedMeshBlob>,
    pub(crate) prepared_runtime_payloads: PreparedRuntimePayloads,
    pub(crate) cache_context: Option<ActiveProjectCacheContext>,
    pub(crate) cache_provenance: Option<DeliveryCacheProvenance>,
}

#[derive(Debug)]
struct DeliveryWork {
    identity: RuntimeDeliveryIdentity,
    project_root: PathBuf,
    snapshot: Arc<SemanticSnapshot>,
    prepared_blobs: Vec<PreparedMeshBlob>,
    prepared_runtime_payloads: PreparedRuntimePayloads,
    profile: RuntimeProfile,
    cache_context: Option<ActiveProjectCacheContext>,
    cache_provenance: Option<DeliveryCacheProvenance>,
}

#[derive(Debug)]
pub(crate) struct DeliveryResult {
    pub(crate) identity: RuntimeDeliveryIdentity,
    pub(crate) bundle: Result<RuntimeDeliveryBundle, String>,
    pub(crate) worker_ms: f64,
    pub(crate) blob_reads: u64,
    pub(crate) bytes: u64,
    pub(crate) prepared_runtime_payloads: PreparedRuntimePayloads,
    pub(crate) cache_context: Option<ActiveProjectCacheContext>,
    pub(crate) cache_provenance: Option<DeliveryCacheProvenance>,
}

#[derive(Debug, Default)]
struct DeliveryQueueState {
    pending: VecDeque<DeliveryWork>,
    closed: bool,
    high_water: u64,
    coalesced: u64,
}

#[derive(Debug)]
struct DeliveryQueue {
    state: Mutex<DeliveryQueueState>,
    wake: Condvar,
}

impl DeliveryQueue {
    fn new() -> Self {
        Self {
            state: Mutex::new(DeliveryQueueState::default()),
            wake: Condvar::new(),
        }
    }

    fn submit(&self, work: DeliveryWork) -> Result<(), Box<DeliveryWork>> {
        let Ok(mut state) = self.state.lock() else {
            return Err(Box::new(work));
        };
        if state.closed {
            return Err(Box::new(work));
        }
        if state.pending.len() >= DELIVERY_QUEUE_CAPACITY {
            // Complete deliveries are derived state. Discard only work that
            // has not started, retaining the newest authoritative snapshot.
            state.pending.pop_front();
            state.coalesced += 1;
        }
        state.pending.push_back(work);
        state.high_water = state.high_water.max(state.pending.len() as u64);
        self.wake.notify_one();
        Ok(())
    }

    fn pop(&self) -> Option<DeliveryWork> {
        let mut state = self.state.lock().ok()?;
        loop {
            if let Some(work) = state.pending.pop_front() {
                return Some(work);
            }
            if state.closed {
                return None;
            }
            state = self.wake.wait(state).ok()?;
        }
    }

    fn stats(&self) -> (u64, u64, u64) {
        self.state.lock().map_or((0, 0, 0), |state| {
            (
                state.pending.len() as u64,
                state.high_water,
                state.coalesced,
            )
        })
    }
}

impl Drop for DeliveryQueue {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            self.wake.notify_all();
        }
    }
}

/// Bevy-facing runtime delivery state. The only data crossing into the worker
/// is an owned semantic snapshot and immutable prepared bytes.
#[derive(Resource)]
pub(crate) struct RuntimeDeliveryRuntime {
    queue: Arc<DeliveryQueue>,
    results: Mutex<mpsc::Receiver<DeliveryResult>>,
    result_backpressure: Arc<AtomicU64>,
    pub(crate) pending: Option<PendingRuntimeDelivery>,
}

impl Default for RuntimeDeliveryRuntime {
    fn default() -> Self {
        let queue = Arc::new(DeliveryQueue::new());
        let (result_sender, result_receiver) = mpsc::sync_channel(DELIVERY_RESULT_CAPACITY);
        let worker_queue = Arc::clone(&queue);
        let result_backpressure = Arc::new(AtomicU64::new(0));
        let worker_result_backpressure = Arc::clone(&result_backpressure);
        thread::Builder::new()
            .name("usdview-runtime-delivery-worker".to_owned())
            .spawn(move || delivery_worker(worker_queue, result_sender, worker_result_backpressure))
            .expect("runtime delivery worker should start");
        Self {
            queue,
            results: Mutex::new(result_receiver),
            result_backpressure,
            pending: None,
        }
    }
}

impl RuntimeDeliveryRuntime {
    pub(crate) fn replace_pending(&mut self, pending: PendingRuntimeDelivery) {
        self.pending = Some(pending);
    }

    pub(crate) fn submit_pending(
        &mut self,
        project_root: &Path,
        cache_context: Option<ActiveProjectCacheContext>,
    ) -> bool {
        let Some(pending) = self.pending.take() else {
            return false;
        };
        let work = DeliveryWork {
            identity: pending.identity,
            project_root: project_root.to_path_buf(),
            snapshot: pending.snapshot,
            prepared_blobs: pending.prepared_blobs,
            prepared_runtime_payloads: pending.prepared_runtime_payloads,
            profile: RuntimeProfile::NativeMedium,
            cache_context,
            cache_provenance: pending.cache_provenance,
        };
        match self.queue.submit(work) {
            Ok(()) => true,
            Err(work) => {
                let work = *work;
                self.pending = Some(PendingRuntimeDelivery {
                    identity: work.identity,
                    snapshot: work.snapshot,
                    prepared_blobs: work.prepared_blobs,
                    prepared_runtime_payloads: work.prepared_runtime_payloads,
                    cache_context: work.cache_context,
                    cache_provenance: work.cache_provenance,
                });
                false
            }
        }
    }

    pub(crate) fn drain_results(&self) -> Vec<DeliveryResult> {
        let Ok(results) = self.results.lock() else {
            return Vec::new();
        };
        results.try_iter().collect()
    }

    pub(crate) fn take_result_backpressure(&self) -> u64 {
        self.result_backpressure.swap(0, Ordering::AcqRel)
    }

    pub(crate) fn queue_stats(&self) -> (u64, u64, u64) {
        self.queue.stats()
    }
}

fn delivery_worker(
    queue: Arc<DeliveryQueue>,
    results: mpsc::SyncSender<DeliveryResult>,
    result_backpressure: Arc<AtomicU64>,
) {
    while let Some(work) = queue.pop() {
        let started = Instant::now();
        let outcome = build_delivery(&work);
        let (blob_reads, bytes) = outcome.as_ref().map_or((0, 0), |bundle| {
            (
                bundle.manifest.meshes.len() as u64,
                bundle
                    .blobs
                    .iter()
                    .map(|(_, bytes)| bytes.len() as u64)
                    .sum(),
            )
        });
        let result = DeliveryResult {
            identity: work.identity,
            bundle: outcome.map_err(|error| format!("{error:#}")),
            worker_ms: started.elapsed().as_secs_f64() * 1000.0,
            blob_reads,
            bytes,
            prepared_runtime_payloads: work.prepared_runtime_payloads,
            cache_context: work.cache_context,
            cache_provenance: work.cache_provenance,
        };
        if !send_delivery_result(&results, result, &result_backpressure) {
            break;
        }
    }
}

fn send_delivery_result(
    results: &mpsc::SyncSender<DeliveryResult>,
    result: DeliveryResult,
    result_backpressure: &AtomicU64,
) -> bool {
    match results.try_send(result) {
        Ok(()) => true,
        Err(mpsc::TrySendError::Full(result)) => {
            result_backpressure.fetch_add(1, Ordering::Relaxed);
            results.send(result).is_ok()
        }
        Err(mpsc::TrySendError::Disconnected(_)) => false,
    }
}

fn build_delivery(work: &DeliveryWork) -> anyhow::Result<RuntimeDeliveryBundle> {
    let store = FilesystemBlobStore::new(work.project_root.join(OBJECTS_DIRECTORY))?;
    let mut persisted = HashSet::with_capacity(
        work.prepared_blobs.len()
            + work.prepared_runtime_payloads.materials.len()
            + work.prepared_runtime_payloads.textures.len(),
    );
    for prepared in &work.prepared_blobs {
        if !persisted.insert(prepared.blob_id.0.clone()) {
            continue;
        }
        let stored = store.put(&prepared.bytes)?;
        anyhow::ensure!(
            stored == prepared.blob_id,
            "prepared mesh digest {} was stored as {}",
            prepared.blob_id.0,
            stored.0
        );
    }
    for prepared in work
        .prepared_runtime_payloads
        .materials
        .iter()
        .chain(work.prepared_runtime_payloads.textures.iter())
    {
        persist_runtime_blob(&store, prepared, &mut persisted)?;
    }
    let bundle = build_runtime_delivery_with_payloads(
        &store,
        &work.snapshot,
        work.profile,
        &work.prepared_runtime_payloads,
    )?;
    if let Some(context) = &work.cache_context {
        if work.prepared_runtime_payloads.complete {
            if let Some(provenance) = work.cache_provenance {
                let descriptor = crate::cache::ProjectCacheDescriptor::new(
                    context.identity.clone(),
                    crate::cache::ProjectCacheState::Ready,
                    Some(bundle.manifest.clone()),
                )?;
                let cache_store = crate::cache::ProjectCacheStore::new(&context.project_root);
                match cache_store.publish_generation(
                    &descriptor,
                    provenance.generation,
                    provenance.revision_digest,
                ) {
                    Ok(true) => {}
                    Ok(false) => {
                        bevy::log::debug!(
                            "[project-cache] delivery superseded, retired, or source changed before publication; ready descriptor suppressed"
                        );
                    }
                    Err(error) => {
                        bevy::log::warn!("[project-cache] ready descriptor publish failed: {error:#}");
                    }
                }
            } else {
                bevy::log::debug!(
                    "[project-cache] delivery has no qualified sealed provenance; ready descriptor suppressed"
                );
            }
        } else {
            bevy::log::debug!(
                "[project-cache] runtime payload coverage is incomplete; ready descriptor suppressed"
            );
        }
    }
    Ok(bundle)
}

fn persist_runtime_blob(
    store: &FilesystemBlobStore,
    prepared: &PreparedRuntimeBlob,
    persisted: &mut HashSet<String>,
) -> anyhow::Result<()> {
    if !persisted.insert(prepared.blob_id.0.clone()) {
        return Ok(());
    }
    let stored = store.put(&prepared.bytes)?;
    anyhow::ensure!(
        stored == prepared.blob_id,
        "prepared runtime digest {} was stored as {}",
        prepared.blob_id.0,
        stored.0
    );
    Ok(())
}

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/semantic/sync/delivery_worker_tests.rs"]
mod tests;
