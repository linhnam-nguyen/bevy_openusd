//! Bounded cache-warm queue ownership and worker lifecycle.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use viewport_protocol::RuntimeProfile;

const WARM_QUEUE_CAPACITY: usize = 2;

/// A bounded queue that coalesces duplicate target warms before they reach the
/// worker. Warm failures are diagnostic and never fail the source mutation.
#[derive(Clone)]
pub struct ProjectCacheWarmQueue {
    state: Arc<WarmQueueState>,
}

struct WarmQueueState {
    sender: Mutex<Option<mpsc::SyncSender<super::WarmJob>>>,
    pending: Arc<Mutex<HashSet<(PathBuf, String)>>>,
    idle: Arc<Condvar>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Default for ProjectCacheWarmQueue {
    fn default() -> Self {
        let (sender, receiver) = mpsc::sync_channel(WARM_QUEUE_CAPACITY);
        let pending = Arc::new(Mutex::new(HashSet::new()));
        let idle = Arc::new(Condvar::new());
        let worker_pending = Arc::clone(&pending);
        let worker_idle = Arc::clone(&idle);
        let worker = std::thread::Builder::new()
            .name("usdhub-project-cache-warm".to_owned())
            .spawn(move || worker_loop(receiver, worker_pending, worker_idle))
            .expect("Project cache warm worker must start");
        let state = WarmQueueState {
            sender: Mutex::new(Some(sender)),
            pending,
            idle,
            worker: Mutex::new(Some(worker)),
        };
        Self {
            state: Arc::new(state),
        }
    }
}

impl Drop for ProjectCacheWarmQueue {
    fn drop(&mut self) {
        if Arc::strong_count(&self.state) != 1 {
            return;
        }
        self.state
            .sender
            .lock()
            .expect("Project cache warm sender is not poisoned")
            .take();
        if let Some(worker) = self
            .state
            .worker
            .lock()
            .expect("Project cache warm worker handle is not poisoned")
            .take()
        {
            // Dropping the last handle detaches the worker. The sender was
            // closed above, so it exits after its current atomic job without
            // making a request-scoped service destructor wait for the build.
            drop(worker);
        }
    }
}

impl ProjectCacheWarmQueue {
    /// Close future warm submissions and detach the worker without waiting for
    /// an in-flight cache build. Runtime owners use this during teardown so a
    /// shared queue clone cannot keep an idle worker alive forever.
    pub(crate) fn shutdown_without_waiting(&self) {
        self.state
            .sender
            .lock()
            .expect("Project cache warm sender is not poisoned")
            .take();
        if let Some(worker) = self
            .state
            .worker
            .lock()
            .expect("Project cache warm worker handle is not poisoned")
            .take()
        {
            drop(worker);
        }
    }

    /// Try to schedule one canonical Project target without blocking the
    /// caller that just published authoritative Project state.
    pub fn enqueue(&self, project_root: &Path, target: super::ProjectCacheTarget) -> bool {
        self.enqueue_targets(project_root, vec![target])
    }

    /// Try to schedule one bounded batch of canonical targets. One import may
    /// therefore warm every Stage-bearing target without filling the queue
    /// with one unbounded request per Scene or Model.
    pub fn enqueue_targets(
        &self,
        project_root: &Path,
        targets: Vec<super::ProjectCacheTarget>,
    ) -> bool {
        let project_root = project_root.to_path_buf();
        let mut targets = targets;
        targets.sort_by_key(super::ProjectCacheTarget::key);
        targets.dedup_by_key(|target| target.key());
        let config_hash = super::super::cache_compatibility::project_runtime_cache_config_hash(
            usd_semantic::SemanticConfig::default().hash(),
        );
        let mut warm_targets = Vec::with_capacity(targets.len());
        for target in targets {
            let identity = match super::ProjectCacheIdentity::for_project(
                &project_root,
                target.clone(),
                RuntimeProfile::NativeMedium,
                config_hash,
            ) {
                Ok(identity) => identity,
                Err(error) => {
                    log::warn!(
                        "Project cache warm identity could not be established for {}: {error:#}",
                        project_root.display()
                    );
                    return false;
                }
            };
            warm_targets.push(super::WarmTarget { target, identity });
        }
        if warm_targets.is_empty() {
            return true;
        }
        let batch_key = warm_targets
            .iter()
            .map(|target| identity_key(&target.identity))
            .collect::<Vec<_>>()
            .join(",");
        let key = (project_root.clone(), format!("batch:{batch_key}"));
        let mut pending = self
            .state
            .pending
            .lock()
            .expect("Project cache warm state is not poisoned");
        if !pending.insert(key.clone()) {
            return true;
        }
        let job = super::WarmJob {
            project_root,
            targets: warm_targets,
            key: key.clone(),
        };
        let sender = self
            .state
            .sender
            .lock()
            .expect("Project cache warm sender is not poisoned");
        let sent = sender
            .as_ref()
            .is_some_and(|sender| sender.try_send(job).is_ok());
        if !sent {
            pending.remove(&key);
            return false;
        }
        true
    }

    /// Wait until no warm job for this Project can publish into its cache.
    /// This is used only at destructive lifecycle boundaries; normal enqueue
    /// and renderer activation remain non-blocking.
    pub(crate) fn wait_for_project_idle(&self, project_root: &Path, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut pending = self
            .state
            .pending
            .lock()
            .expect("Project cache warm state is not poisoned");
        loop {
            if !pending.iter().any(|(root, _)| root == project_root) {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (next, wait) = self
                .state
                .idle
                .wait_timeout(pending, remaining)
                .expect("Project cache warm state is not poisoned");
            pending = next;
            if wait.timed_out() {
                return !pending.iter().any(|(root, _)| root == project_root);
            }
        }
    }
}

fn worker_loop(
    receiver: mpsc::Receiver<super::WarmJob>,
    pending: Arc<Mutex<HashSet<(PathBuf, String)>>>,
    idle: Arc<Condvar>,
) {
    while let Ok(job) = receiver.recv() {
        let _ = super::warm_job(&job);
        let mut pending = pending
            .lock()
            .expect("Project cache warm state is not poisoned");
        pending.remove(&job.key);
        idle.notify_all();
    }
}

fn identity_key(identity: &super::ProjectCacheIdentity) -> String {
    let bytes = serde_json::to_vec(identity).expect("Project cache identity is serializable");
    blake3::hash(&bytes).to_hex().to_string()
}
