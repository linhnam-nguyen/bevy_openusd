use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Instant;

use bevy::prelude::Resource;
use usd_bevy::LiveStage;

use super::recovery::{RecoveryCheckpointWork, RecoveryMetadata, RecoveryStore};

const RECOVERY_QUEUE_CAPACITY: usize = 4;
const RECOVERY_RESULT_CAPACITY: usize = 4;

#[derive(Debug)]
struct RecoveryResult {
    session_id: u64,
    live_revision: u64,
    result: anyhow::Result<RecoveryMetadata>,
    worker_ms: f64,
}

#[derive(Debug)]
struct RecoveryQueue {
    state: Mutex<RecoveryQueueState>,
    wake: Condvar,
}

#[derive(Debug, Default)]
struct RecoveryQueueState {
    pending: VecDeque<RecoveryCheckpointWork>,
    closed: bool,
    high_water: u64,
    coalesced: u64,
}

impl RecoveryQueue {
    fn new() -> Self {
        Self {
            state: Mutex::new(RecoveryQueueState::default()),
            wake: Condvar::new(),
        }
    }

    fn submit(&self, work: RecoveryCheckpointWork) -> Result<(), RecoveryCheckpointWork> {
        let Ok(mut state) = self.state.lock() else {
            return Err(work);
        };
        if state.closed {
            return Err(work);
        }
        if state.pending.len() >= RECOVERY_QUEUE_CAPACITY {
            state.pending.pop_front();
            state.coalesced += 1;
        }
        state.pending.push_back(work);
        state.high_water = state.high_water.max(state.pending.len() as u64);
        self.wake.notify_one();
        Ok(())
    }

    fn recv(&self) -> Option<RecoveryCheckpointWork> {
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

    fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            self.wake.notify_all();
        }
    }
}

/// Persistent worker runtime for recovery filesystem operations.
#[derive(Resource, Debug)]
pub(crate) struct RecoveryRuntime {
    queue: Arc<RecoveryQueue>,
    results: Mutex<mpsc::Receiver<RecoveryResult>>,
    result_backpressure: Arc<AtomicU64>,
}

impl Default for RecoveryRuntime {
    fn default() -> Self {
        let queue = Arc::new(RecoveryQueue::new());
        let (pending_results, results) = mpsc::sync_channel(RECOVERY_RESULT_CAPACITY);
        let worker_queue = Arc::clone(&queue);
        let result_backpressure = Arc::new(AtomicU64::new(0));
        let worker_result_backpressure = Arc::clone(&result_backpressure);
        std::thread::Builder::new()
            .name("usdview-recovery-worker".to_owned())
            .spawn(move || {
                recovery_worker(worker_queue, pending_results, worker_result_backpressure)
            })
            .expect("recovery worker should start");
        Self {
            queue,
            results: Mutex::new(results),
            result_backpressure,
        }
    }
}

impl RecoveryRuntime {
    pub(crate) fn submit(&self, work: RecoveryCheckpointWork) -> bool {
        self.queue.submit(work).is_ok()
    }

    pub(crate) fn stats(&self) -> (u64, u64, u64) {
        self.queue.stats()
    }

    fn take_result_backpressure(&self) -> u64 {
        self.result_backpressure.swap(0, Ordering::AcqRel)
    }

    fn drain_results(&self) -> Vec<RecoveryResult> {
        self.results
            .lock()
            .map_or_else(|_| Vec::new(), |results| results.try_iter().collect())
    }
}

impl Drop for RecoveryRuntime {
    fn drop(&mut self) {
        self.queue.close();
    }
}

fn recovery_worker(
    queue: Arc<RecoveryQueue>,
    results: mpsc::SyncSender<RecoveryResult>,
    result_backpressure: Arc<AtomicU64>,
) {
    while let Some(work) = queue.recv() {
        let started = Instant::now();
        let result = RecoveryStore::new(&work.project_root, work.session_id).and_then(|store| {
            store.write_checkpoint_bytes(
                &work.stage_bytes,
                work.live_revision,
                std::path::Path::new(&work.source_stage),
                work.base_revision.as_deref(),
            )
        });
        let result = RecoveryResult {
            session_id: work.session_id,
            live_revision: work.live_revision,
            result,
            worker_ms: started.elapsed().as_secs_f64() * 1000.0,
        };
        if !send_recovery_result(&results, result, &result_backpressure) {
            break;
        }
    }
}

fn send_recovery_result(
    results: &mpsc::SyncSender<RecoveryResult>,
    result: RecoveryResult,
    result_backpressure: &AtomicU64,
) -> bool {
    send_recovery_result_with_hook(results, result, result_backpressure, || {})
}

fn send_recovery_result_with_hook<F>(
    results: &mpsc::SyncSender<RecoveryResult>,
    result: RecoveryResult,
    result_backpressure: &AtomicU64,
    on_full: F,
) -> bool
where
    F: FnOnce(),
{
    match results.try_send(result) {
        Ok(()) => true,
        Err(mpsc::TrySendError::Full(result)) => {
            result_backpressure.fetch_add(1, Ordering::Relaxed);
            on_full();
            results.send(result).is_ok()
        }
        Err(mpsc::TrySendError::Disconnected(_)) => false,
    }
}

pub(crate) fn drain_recovery_results(
    runtime: Option<bevy::ecs::system::Res<RecoveryRuntime>>,
    stage: Option<bevy::ecs::system::NonSend<LiveStage>>,
    mut counters: Option<
        bevy::ecs::system::ResMut<crate::viewport::diagnostics::performance::RendererCounters>,
    >,
) {
    let Some(runtime) = runtime else {
        return;
    };
    let result_backpressure = runtime.take_result_backpressure();
    if let Some(ref mut counters) = counters {
        counters.recovery_result_backpressure += result_backpressure;
    }
    for result in runtime.drain_results() {
        if let Some(ref mut counters) = counters {
            counters.recovery_worker_write_ms += result.worker_ms;
        }
        let current_identity = stage
            .as_ref()
            .map(|stage| (stage.session_id(), stage.current_revision().0));
        if current_identity.is_some_and(|(session_id, revision)| {
            session_id != result.session_id || revision < result.live_revision
        }) {
            continue;
        }
        match result.result {
            Ok(_) => {
                if let Some(ref mut counters) = counters {
                    counters.recovery_successes += 1;
                }
            }
            Err(error) => {
                bevy::log::error!(
                    "[recovery] worker checkpoint failed at revision {}: {error:#}",
                    result.live_revision
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/project/recovery_worker_tests.rs"]
mod tests;
