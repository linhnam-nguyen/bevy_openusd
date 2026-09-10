//! Bounded cache-warm queue ownership and worker lifecycle.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, mpsc,
        atomic::AtomicU64,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[cfg(test)]
use anyhow::Result;
use viewport_protocol::RuntimeProfile;

#[path = "cache_warmer_enqueue.rs"]
mod enqueue;

const WARM_QUEUE_CAPACITY: usize = 2;
pub(super) const WARM_LATEST_CAPACITY: usize = 8;
pub(super) const WARM_DIRTY_PROJECT_CAPACITY: usize = 4;
const CACHE_PREPARATION_POLL: Duration = Duration::from_millis(5);
type WarmKey = (PathBuf, String);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProjectCachePreparation {
    Ready,
    Empty,
    FallbackRequired,
}

#[derive(Clone)]
pub struct ProjectCacheWarmQueue {
    state: Arc<WarmQueueState>,
}

struct WarmQueueState {
    sender: Arc<Mutex<Option<mpsc::SyncSender<super::WarmJob>>>>,
    latest: Arc<LatestWarmState>,
    worker: Mutex<Option<JoinHandle<()>>>,
    next_build_generation: AtomicU64,
}

pub(super) struct LatestWarmState {
    inner: Mutex<LatestWarmInner>,
    idle: Condvar,
}

#[derive(Default)]
struct LatestWarmInner {
    latest: HashMap<WarmKey, super::WarmTarget>,
    scheduled: HashSet<WarmKey>,
    dirty_projects: BTreeSet<PathBuf>,
}

impl LatestWarmState {
    pub(super) fn new() -> Self {
        Self { inner: Mutex::new(LatestWarmInner::default()), idle: Condvar::new() }
    }

    pub(super) fn record(
        &self,
        key: WarmKey,
        target: super::WarmTarget,
        sender: &mpsc::SyncSender<super::WarmJob>,
    ) -> bool {
        let mut inner = self.inner.lock().expect("Project cache warm state is not poisoned");
        loop {
            let known = inner.latest.contains_key(&key) || inner.scheduled.contains(&key);
            if !known && inner.latest.len() >= WARM_LATEST_CAPACITY {
                if target.scene_generation.is_none() { return false; }
                let represented = inner.dirty_projects.contains(&key.0)
                    || inner.dirty_projects.len() < WARM_DIRTY_PROJECT_CAPACITY;
                if represented {
                    inner.dirty_projects.insert(key.0.clone());
                    self.idle.notify_all();
                    return true;
                }
                inner = self.idle.wait(inner).expect("Project cache warm state is not poisoned");
                continue;
            }
            inner.latest.insert(key.clone(), target);
            if inner.scheduled.contains(&key) { return true; }
            inner.scheduled.insert(key.clone());
            break;
        }
        drop(inner);
        match sender.try_send(super::WarmJob { key: key.clone() }) {
            Ok(()) => true,
            Err(mpsc::TrySendError::Full(_)) => {
                self.inner.lock().expect("Project cache warm state is not poisoned").scheduled.remove(&key);
                true
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                let mut inner = self.inner.lock().expect("Project cache warm state is not poisoned");
                inner.scheduled.remove(&key);
                inner.latest.remove(&key);
                inner.dirty_projects.remove(&key.0);
                false
            }
        }
    }

    pub(super) fn take_latest(&self, key: &WarmKey) -> Option<super::WarmTarget> {
        let target = self.inner.lock().expect("Project cache warm state is not poisoned").latest.remove(key);
        if target.is_some() { self.idle.notify_all(); }
        target
    }

    pub(super) fn cancel(&self, key: &WarmKey) {
        self.inner.lock().expect("Project cache warm state is not poisoned").latest.remove(key);
        self.idle.notify_all();
    }

    pub(super) fn finish_and_retry(
        &self,
        key: &WarmKey,
        sender: Option<&mpsc::SyncSender<super::WarmJob>>,
    ) {
        self.inner.lock().expect("Project cache warm state is not poisoned").scheduled.remove(key);
        self.idle.notify_all();
        if let Some(sender) = sender { self.retry_unscheduled(sender); }
    }

    fn retry_unscheduled(&self, sender: &mpsc::SyncSender<super::WarmJob>) {
        loop {
            let next = {
                let mut inner = self.inner.lock().expect("Project cache warm state is not poisoned");
                let next = inner.latest.keys().filter(|key| !inner.scheduled.contains(*key)).min().cloned();
                if let Some(key) = &next { inner.scheduled.insert(key.clone()); }
                next
            };
            if let Some(key) = next {
                match sender.try_send(super::WarmJob { key: key.clone() }) {
                    Ok(()) => continue,
                    Err(mpsc::TrySendError::Full(_)) => {
                        self.inner.lock().expect("Project cache warm state is not poisoned").scheduled.remove(&key);
                        return;
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => {
                        let mut inner = self.inner.lock().expect("Project cache warm state is not poisoned");
                        inner.latest.clear();
                        inner.scheduled.clear();
                        inner.dirty_projects.clear();
                        return;
                    }
                }
            }

            let project_root = {
                let mut inner = self.inner.lock().expect("Project cache warm state is not poisoned");
                if inner.latest.len() >= WARM_LATEST_CAPACITY { return; }
                let next = inner.dirty_projects.iter().next().cloned();
                if let Some(root) = &next { inner.dirty_projects.remove(root); }
                next
            };
            let Some(project_root) = project_root else { return; };
            let recovered = recover_building_scene_targets(&project_root);
            let mut inner = self.inner.lock().expect("Project cache warm state is not poisoned");
            let mut overflow = false;
            for target in recovered {
                let key = (project_root.clone(), target.target.key());
                if inner.latest.contains_key(&key) || inner.scheduled.contains(&key) { continue; }
                if inner.latest.len() >= WARM_LATEST_CAPACITY {
                    overflow = true;
                    break;
                }
                inner.latest.insert(key, target);
            }
            if overflow && (inner.dirty_projects.contains(&project_root)
                || inner.dirty_projects.len() < WARM_DIRTY_PROJECT_CAPACITY)
            {
                inner.dirty_projects.insert(project_root);
            }
        }
    }

    fn wait_for_project_idle(&self, project_root: &Path, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut inner = self.inner.lock().expect("Project cache warm state is not poisoned");
        loop {
            let busy = inner.latest.keys().any(|(root, _)| root == project_root)
                || inner.scheduled.iter().any(|(root, _)| root == project_root)
                || inner.dirty_projects.contains(project_root);
            if !busy { return true; }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { return false; }
            let (next, wait) = self.idle.wait_timeout(inner, remaining).expect("Project cache warm state is not poisoned");
            inner = next;
            if wait.timed_out() {
                return !inner.latest.keys().any(|(root, _)| root == project_root)
                    && !inner.scheduled.iter().any(|(root, _)| root == project_root)
                    && !inner.dirty_projects.contains(project_root);
            }
        }
    }

    #[cfg(test)]
    pub(super) fn retained_counts(&self) -> (usize, usize) {
        let inner = self.inner.lock().expect("Project cache warm state is not poisoned");
        (inner.latest.len(), inner.dirty_projects.len())
    }
}

fn recover_building_scene_targets(project_root: &Path) -> Vec<super::WarmTarget> {
    let manifest = match super::ManifestStore::read_validated(project_root) {
        Ok(manifest) => manifest,
        Err(_) => return Vec::new(),
    };
    let store = super::SceneCacheStore::new(project_root);
    let mut targets = Vec::new();
    for scene in manifest.scenes() {
        let Ok(Some(descriptor)) = store.load_descriptor(scene.id) else { continue; };
        if descriptor.state != super::SceneCacheState::Building { continue; }
        targets.push(super::WarmTarget {
            target: super::ProjectCacheTarget::Scene { id: scene.id.to_string() },
            scene_generation: Some(descriptor.generation),
            build_generation: 0,
        });
    }
    targets.sort_by_key(|target| target.target.key());
    targets
}

impl Default for ProjectCacheWarmQueue {
    fn default() -> Self {
        let (sender, receiver) = mpsc::sync_channel(WARM_QUEUE_CAPACITY);
        let sender = Arc::new(Mutex::new(Some(sender)));
        let latest = Arc::new(LatestWarmState::new());
        let worker_latest = Arc::clone(&latest);
        let worker_sender = Arc::clone(&sender);
        let worker = std::thread::Builder::new()
            .name("usdhub-project-cache-warm".to_owned())
            .spawn(move || worker_loop(receiver, worker_sender, worker_latest))
            .expect("Project cache warm worker must start");
        Self {
            state: Arc::new(WarmQueueState {
                sender,
                latest,
                worker: Mutex::new(Some(worker)),
                next_build_generation: AtomicU64::new(1),
            }),
        }
    }
}

impl Drop for ProjectCacheWarmQueue {
    fn drop(&mut self) {
        if Arc::strong_count(&self.state) != 1 { return; }
        self.state.sender.lock().expect("Project cache warm sender is not poisoned").take();
        if let Some(worker) = self.state.worker.lock().expect("Project cache warm worker handle is not poisoned").take() {
            drop(worker);
        }
    }
}

impl ProjectCacheWarmQueue {
    pub(crate) fn shutdown_without_waiting(&self) {
        self.state.sender.lock().expect("Project cache warm sender is not poisoned").take();
        if let Some(worker) = self.state.worker.lock().expect("Project cache warm worker handle is not poisoned").take() {
            drop(worker);
        }
    }

    pub(super) fn cancel_target(&self, project_root: &Path, target: &super::ProjectCacheTarget) {
        self.state.latest.cancel(&(project_root.to_path_buf(), target.key()));
    }

    pub(crate) fn wait_for_project_idle(&self, project_root: &Path, timeout: Duration) -> bool {
        self.state.latest.wait_for_project_idle(project_root, timeout)
    }

    pub(crate) fn prepare_for_activation(
        &self,
        project_root: &Path,
        target: super::ProjectCacheTarget,
    ) -> ProjectCachePreparation {
        let store = super::ProjectCacheStore::new(project_root);
        let identity = match super::ProjectCacheIdentity::for_project(
            project_root,
            target,
            RuntimeProfile::NativeMedium,
            super::super::cache_compatibility::project_runtime_cache_config_hash(
                usd_semantic::SemanticConfig::default().hash(),
            ),
        ) {
            Ok(identity) => identity,
            Err(error) => {
                log::warn!("Project cache activation identity could not be established for {}: {error:#}", project_root.display());
                return ProjectCachePreparation::FallbackRequired;
            }
        };
        Self::probe_for_activation(&store, &identity)
    }

    pub(crate) fn probe_for_activation(
        store: &super::ProjectCacheStore,
        identity: &super::ProjectCacheIdentity,
    ) -> ProjectCachePreparation {
        match store.load(identity) {
            Ok(Some(descriptor)) => match descriptor.state {
                super::ProjectCacheState::Ready => ProjectCachePreparation::Ready,
                super::ProjectCacheState::Empty => ProjectCachePreparation::Empty,
                super::ProjectCacheState::FallbackRequired
                | super::ProjectCacheState::Building
                | super::ProjectCacheState::Partial => ProjectCachePreparation::FallbackRequired,
            },
            Ok(None) => ProjectCachePreparation::FallbackRequired,
            Err(error) => {
                log::warn!("Project cache activation descriptor is unavailable for {}: {error:#}", identity.target.key());
                ProjectCachePreparation::FallbackRequired
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn wait_for(
    _queue: &ProjectCacheWarmQueue,
    project_root: &Path,
    target: &super::ProjectCacheTarget,
) -> Result<Option<super::ProjectCacheDescriptor>> {
    let identity = super::ProjectCacheIdentity::for_project(
        project_root,
        target.clone(),
        RuntimeProfile::NativeMedium,
        super::super::cache_compatibility::project_runtime_cache_config_hash(
            usd_semantic::SemanticConfig::default().hash(),
        ),
    )?;
    let store = super::ProjectCacheStore::new(project_root);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(descriptor) = store.load(&identity)? {
            if descriptor.state != super::ProjectCacheState::Building { return Ok(Some(descriptor)); }
        }
        if Instant::now() >= deadline { return Ok(None); }
        std::thread::sleep(CACHE_PREPARATION_POLL);
    }
}

fn worker_loop(
    receiver: mpsc::Receiver<super::WarmJob>,
    sender: Arc<Mutex<Option<mpsc::SyncSender<super::WarmJob>>>>,
    latest: Arc<LatestWarmState>,
) {
    while let Ok(job) = receiver.recv() {
        if let Some(target) = latest.take_latest(&job.key) {
            if let Err(error) = super::warm_target(&job.key.0, &target) {
                log::warn!("Project cache warm failed for {} ({}): {error:#}", job.key.0.display(), target.target.key());
            }
        }
        let sender = sender.lock().expect("Project cache warm sender is not poisoned").clone();
        latest.finish_and_retry(&job.key, sender.as_ref());
    }
}
