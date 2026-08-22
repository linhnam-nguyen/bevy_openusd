use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::Instant;

use bevy::prelude::Resource;
use usd_bevy::LiveRevision;
use usd_model::SemanticSnapshot;
use viewport_protocol::RuntimeProfile;

use crate::project::blob_store::{
    BlobStore, FilesystemBlobStore, OBJECTS_DIRECTORY, PreparedMeshBlob,
};
use crate::project::runtime_delivery::{RuntimeDeliveryBundle, build_runtime_delivery};

pub(crate) const DELIVERY_QUEUE_CAPACITY: usize = 4;
pub(crate) const DELIVERY_RESULT_CAPACITY: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeDeliveryIdentity {
    pub(crate) session_id: u64,
    pub(crate) live_revision: LiveRevision,
    pub(crate) projection_generation: u64,
}

#[derive(Debug)]
pub(crate) struct PendingRuntimeDelivery {
    pub(crate) identity: RuntimeDeliveryIdentity,
    pub(crate) snapshot: SemanticSnapshot,
    pub(crate) prepared_blobs: Vec<PreparedMeshBlob>,
}

#[derive(Debug)]
struct DeliveryWork {
    identity: RuntimeDeliveryIdentity,
    project_root: PathBuf,
    snapshot: SemanticSnapshot,
    prepared_blobs: Vec<PreparedMeshBlob>,
    profile: RuntimeProfile,
}

#[derive(Debug)]
pub(crate) struct DeliveryResult {
    pub(crate) identity: RuntimeDeliveryIdentity,
    pub(crate) bundle: Result<RuntimeDeliveryBundle, String>,
    pub(crate) worker_ms: f64,
    pub(crate) blob_reads: u64,
    pub(crate) bytes: u64,
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

    fn submit(&self, work: DeliveryWork) -> Result<(), DeliveryWork> {
        let Ok(mut state) = self.state.lock() else {
            return Err(work);
        };
        if state.closed {
            return Err(work);
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
    pub(crate) pending: Option<PendingRuntimeDelivery>,
}

impl Default for RuntimeDeliveryRuntime {
    fn default() -> Self {
        let queue = Arc::new(DeliveryQueue::new());
        let (result_sender, result_receiver) = mpsc::sync_channel(DELIVERY_RESULT_CAPACITY);
        let worker_queue = Arc::clone(&queue);
        thread::Builder::new()
            .name("usdview-runtime-delivery-worker".to_owned())
            .spawn(move || delivery_worker(worker_queue, result_sender))
            .expect("runtime delivery worker should start");
        Self {
            queue,
            results: Mutex::new(result_receiver),
            pending: None,
        }
    }
}

impl RuntimeDeliveryRuntime {
    pub(crate) fn replace_pending(&mut self, pending: PendingRuntimeDelivery) {
        self.pending = Some(pending);
    }

    pub(crate) fn submit_pending(&mut self, project_root: &Path) -> bool {
        let Some(pending) = self.pending.take() else {
            return false;
        };
        let work = DeliveryWork {
            identity: pending.identity,
            project_root: project_root.to_path_buf(),
            snapshot: pending.snapshot,
            prepared_blobs: pending.prepared_blobs,
            profile: RuntimeProfile::NativeMedium,
        };
        match self.queue.submit(work) {
            Ok(()) => true,
            Err(work) => {
                self.pending = Some(PendingRuntimeDelivery {
                    identity: work.identity,
                    snapshot: work.snapshot,
                    prepared_blobs: work.prepared_blobs,
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

    pub(crate) fn queue_stats(&self) -> (u64, u64, u64) {
        self.queue.stats()
    }
}

fn delivery_worker(queue: Arc<DeliveryQueue>, results: mpsc::SyncSender<DeliveryResult>) {
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
        };
        let _ = results.try_send(result);
    }
}

fn build_delivery(work: &DeliveryWork) -> anyhow::Result<RuntimeDeliveryBundle> {
    let store = FilesystemBlobStore::new(work.project_root.join(OBJECTS_DIRECTORY))?;
    let mut persisted = HashSet::with_capacity(work.prepared_blobs.len());
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
    build_runtime_delivery(&store, &work.snapshot, work.profile)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use usd_model::{HashDigest, SnapshotId, SnapshotSource};

    use super::*;

    fn work(revision: u64) -> DeliveryWork {
        DeliveryWork {
            identity: RuntimeDeliveryIdentity {
                session_id: 7,
                live_revision: LiveRevision(revision),
                projection_generation: 3,
            },
            project_root: PathBuf::from("/tmp/h1-delivery-test"),
            snapshot: SemanticSnapshot {
                snapshot_id: SnapshotId(format!("h1-{revision}")),
                source: SnapshotSource::Working {
                    session: "h1-test".to_owned(),
                    live_revision: revision,
                },
                config_hash: HashDigest::new([0; HashDigest::BYTE_LEN]),
                entities: HashMap::new(),
            },
            prepared_blobs: Vec::new(),
            profile: RuntimeProfile::NativeMedium,
        }
    }

    #[test]
    fn delivery_queue_is_bounded_and_keeps_latest_pending_revision() {
        let queue = DeliveryQueue::new();
        for revision in 1..=5 {
            assert!(queue.submit(work(revision)).is_ok());
        }
        let (pending, high_water, coalesced) = queue.stats();
        assert_eq!(pending, DELIVERY_QUEUE_CAPACITY as u64);
        assert_eq!(high_water, DELIVERY_QUEUE_CAPACITY as u64);
        assert_eq!(coalesced, 1);

        let mut revisions = Vec::new();
        for _ in 0..DELIVERY_QUEUE_CAPACITY {
            revisions.push(
                queue
                    .pop()
                    .expect("queued delivery")
                    .identity
                    .live_revision
                    .0,
            );
        }
        assert_eq!(revisions, vec![2, 3, 4, 5]);
    }
}
