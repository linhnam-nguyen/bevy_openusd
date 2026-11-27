use std::{
    path::Path,
    time::{Duration, Instant},
};

use anyhow::Result;
use viewport_protocol::RuntimeProfile;

use super::*;

impl LatestWarmState {
    pub(crate) fn retained_counts(&self) -> (usize, usize) {
        let inner = self
            .inner
            .lock()
            .expect("Project cache warm state is not poisoned");
        (inner.latest.len(), inner.dirty_projects.len())
    }
}

pub(crate) fn wait_for(
    _queue: &ProjectCacheWarmQueue,
    project_root: &Path,
    target: &super::super::ProjectCacheTarget,
) -> Result<Option<super::super::ProjectCacheDescriptor>> {
    let identity = super::super::ProjectCacheIdentity::for_project(
        project_root,
        target.clone(),
        RuntimeProfile::NativeMedium,
        super::super::super::compatibility::project_runtime_cache_config_hash(
            usd_semantic::SemanticConfig::default().hash(),
        ),
    )?;
    let store = super::super::ProjectCacheStore::new(project_root);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(descriptor) = store.load(&identity)? {
            if descriptor.state != super::super::ProjectCacheState::Building {
                return Ok(Some(descriptor));
            }
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(CACHE_PREPARATION_POLL);
    }
}

#[test]
fn final_queue_owner_drops_without_waiting_for_contended_sender() {
    let queue = ProjectCacheWarmQueue::default();
    let sender = Arc::clone(&queue.state.sender);
    let guard = sender.lock().expect("hold sender mutex across last owner drop");
    let worker = queue.state.worker.lock().expect("worker mutex")
        .take().expect("production worker");
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let dropper = std::thread::spawn(move || {
        drop(queue);
        dropped_tx.send(()).expect("report completed destructor");
    });
    let outcome = dropped_rx.recv_timeout(Duration::from_secs(5));
    // Always release the held lock, including on regression failure.
    drop(guard);
    drop(sender);
    dropper.join().expect("queue destructor must not panic");
    assert!(outcome.is_ok(), "queue destruction waited for the sender lock");
    assert_worker_exits(worker);
}

#[test]
fn dropping_nonfinal_queue_owner_preserves_worker_until_last_owner() {
    let queue = ProjectCacheWarmQueue::default();
    let remaining = queue.clone();
    let worker = queue.state.worker.lock().expect("worker mutex")
        .take().expect("production worker");
    drop(queue);
    assert!(!worker.is_finished(), "nonfinal owner stopped the worker");
    drop(remaining);
    assert_worker_exits(worker);
}

#[test]
fn poisoned_sender_does_not_keep_worker_channel_alive_after_final_drop() {
    let queue = ProjectCacheWarmQueue::default();
    let sender = Arc::clone(&queue.state.sender);
    let poison = Arc::clone(&sender);
    assert!(std::thread::spawn(move || {
        let _guard = poison.lock().expect("sender lock");
        panic!("intentional sender poison");
    }).join().is_err());
    assert!(sender.is_poisoned());
    let worker = queue.state.worker.lock().expect("worker mutex")
        .take().expect("production worker");
    drop(sender);
    drop(queue);
    assert_worker_exits(worker);
}

fn assert_worker_exits(worker: JoinHandle<()>) {
    let (exited_tx, exited_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = exited_tx.send(worker.join().is_ok());
    });
    assert_eq!(
        exited_rx.recv_timeout(Duration::from_secs(5)),
        Ok(true),
        "worker must terminate without retaining its own admission sender"
    );
}
