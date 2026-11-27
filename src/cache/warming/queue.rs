//! Bounded cache-warm queue ownership and worker lifecycle.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, Weak, atomic::AtomicU64, mpsc},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use viewport_protocol::RuntimeProfile;

#[path = "enqueue.rs"]
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

const CACHE_MAINTENANCE_KEY: &str = "cache-maintenance";
const WARM_MAINTENANCE_CAPACITY: usize = WARM_DIRTY_PROJECT_CAPACITY;

#[derive(Default)]
struct LatestWarmInner {
    latest: HashMap<WarmKey, super::WarmTarget>,
    scheduled: HashSet<WarmKey>,
    dirty_projects: BTreeSet<PathBuf>,
    maintenance_projects: BTreeSet<PathBuf>,
    failures: HashMap<WarmKey, String>,
}

impl LatestWarmState {
    pub(super) fn new() -> Self {
        Self {
            inner: Mutex::new(LatestWarmInner::default()),
            idle: Condvar::new(),
        }
    }

    pub(super) fn record(
        &self,
        key: WarmKey,
        target: super::WarmTarget,
        sender: &mpsc::SyncSender<super::WarmJob>,
    ) -> bool {
        let mut inner = self
            .inner
            .lock()
            .expect("Project cache warm state is not poisoned");
        loop {
            if inner.latest.get(&key).is_some_and(|current|
                current.scene_generation.zip(target.scene_generation)
                    .is_some_and(|(current, incoming)| current > incoming)) {
                return true;
            }
            inner.failures.remove(&key);
            let known = inner.latest.contains_key(&key) || inner.scheduled.contains(&key);
            if !known && inner.latest.len() >= WARM_LATEST_CAPACITY {
                if target.scene_generation.is_none() {
                    return false;
                }
                let represented = inner.dirty_projects.contains(&key.0)
                    || inner.dirty_projects.len() < WARM_DIRTY_PROJECT_CAPACITY;
                if represented {
                    inner.dirty_projects.insert(key.0.clone());
                    self.idle.notify_all();
                    return true;
                }
                inner = self
                    .idle
                    .wait(inner)
                    .expect("Project cache warm state is not poisoned");
                continue;
            }
            inner.latest.insert(key.clone(), target);
            if inner.scheduled.contains(&key) {
                return true;
            }
            inner.scheduled.insert(key.clone());
            break;
        }
        drop(inner);
        match sender.try_send(super::WarmJob { key: key.clone() }) {
            Ok(()) => true,
            Err(mpsc::TrySendError::Full(_)) => {
                self.inner
                    .lock()
                    .expect("Project cache warm state is not poisoned")
                    .scheduled
                    .remove(&key);
                true
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                let mut inner = self
                    .inner
                    .lock()
                    .expect("Project cache warm state is not poisoned");
                inner.scheduled.remove(&key);
                inner.latest.remove(&key);
                inner.dirty_projects.remove(&key.0);
                false
            }
        }
    }

    /// Maintenance is advisory and coalesced by the same Project locator used
    /// for warm jobs. Saturation defers to the next lifecycle or startup trigger.
    fn request_maintenance(&self, root: &Path, sender: &mpsc::SyncSender<super::WarmJob>) -> bool {
        let key = (root.to_path_buf(), CACHE_MAINTENANCE_KEY.to_owned());
        let Ok(mut inner) = self.inner.lock() else { return false; };
        if inner.maintenance_projects.contains(root) { return true; }
        if inner.maintenance_projects.len() >= WARM_MAINTENANCE_CAPACITY { return false; }
        inner.maintenance_projects.insert(root.to_path_buf());
        self.idle.notify_all();
        if !inner.scheduled.insert(key.clone()) { return true; }
        // try_send cannot wait for the worker. Holding the state mutex across
        // this nonblocking admission prevents a lost retry when the queue drains.
        match sender.try_send(super::WarmJob { key: key.clone() }) {
            Ok(()) => true,
            Err(mpsc::TrySendError::Full(_)) => {
                inner.scheduled.remove(&key);
                true
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                inner.scheduled.remove(&key);
                inner.maintenance_projects.remove(root);
                self.idle.notify_all();
                false
            }
        }
    }

    fn take_maintenance(&self, root: &Path) -> bool {
        self.inner.lock().is_ok_and(|mut inner| inner.maintenance_projects.remove(root))
    }

    pub(super) fn take_latest(&self, key: &WarmKey) -> Option<super::WarmTarget> {
        let target = self
            .inner
            .lock()
            .expect("Project cache warm state is not poisoned")
            .latest
            .remove(key);
        if target.is_some() {
            self.idle.notify_all();
        }
        target
    }

    pub(super) fn cancel(&self, key: &WarmKey) {
        self.inner
            .lock()
            .expect("Project cache warm state is not poisoned")
            .latest
            .remove(key);
        self.idle.notify_all();
    }

    pub(super) fn finish_and_retry(
        &self,
        key: &WarmKey,
        sender: Option<&mpsc::SyncSender<super::WarmJob>>,
    ) {
        self.inner
            .lock()
            .expect("Project cache warm state is not poisoned")
            .scheduled
            .remove(key);
        self.idle.notify_all();
        if let Some(sender) = sender {
            self.retry_unscheduled(sender);
        }
    }

    pub(super) fn record_failure(&self, key: &WarmKey, error: String) {
        self.inner
            .lock()
            .expect("Project cache warm state is not poisoned")
            .failures
            .insert(key.clone(), error);
        self.idle.notify_all();
    }

    fn failure_for_project(&self, project_root: &Path) -> Option<String> {
        self.inner
            .lock()
            .expect("Project cache warm state is not poisoned")
            .failures
            .iter()
            .find(|(key, _)| key.0 == project_root)
            .map(|(_, error)| error.clone())
    }

    fn retry_unscheduled(&self, sender: &mpsc::SyncSender<super::WarmJob>) {
        loop {
            let next = {
                let mut inner = self
                    .inner
                    .lock()
                    .expect("Project cache warm state is not poisoned");
                let next = inner
                    .latest
                    .keys()
                    .filter(|key| !inner.scheduled.contains(*key))
                    .min()
                    .cloned()
                    .or_else(|| inner.maintenance_projects.iter()
                        .map(|root| (root.clone(), CACHE_MAINTENANCE_KEY.to_owned()))
                        .find(|key| !inner.scheduled.contains(key)));
                if let Some(key) = &next {
                    inner.scheduled.insert(key.clone());
                }
                next
            };
            if let Some(key) = next {
                match sender.try_send(super::WarmJob { key: key.clone() }) {
                    Ok(()) => continue,
                    Err(mpsc::TrySendError::Full(_)) => {
                        self.inner
                            .lock()
                            .expect("Project cache warm state is not poisoned")
                            .scheduled
                            .remove(&key);
                        return;
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => {
                        let mut inner = self
                            .inner
                            .lock()
                            .expect("Project cache warm state is not poisoned");
                        inner.latest.clear();
                        inner.scheduled.clear();
                        inner.dirty_projects.clear();
                        inner.maintenance_projects.clear();
                        return;
                    }
                }
            }

            let project_root = {
                let mut inner = self
                    .inner
                    .lock()
                    .expect("Project cache warm state is not poisoned");
                if inner.latest.len() >= WARM_LATEST_CAPACITY {
                    return;
                }
                let next = inner.dirty_projects.iter().next().cloned();
                if let Some(root) = &next {
                    inner.dirty_projects.remove(root);
                }
                next
            };
            let Some(project_root) = project_root else {
                return;
            };
            let recovered = recover_building_scene_targets(&project_root);
            let mut inner = self
                .inner
                .lock()
                .expect("Project cache warm state is not poisoned");
            let mut overflow = false;
            for target in recovered {
                let key = (project_root.clone(), target.target.key());
                if inner.latest.contains_key(&key) || inner.scheduled.contains(&key) {
                    continue;
                }
                if inner.latest.len() >= WARM_LATEST_CAPACITY {
                    overflow = true;
                    break;
                }
                inner.latest.insert(key, target);
            }
            if overflow
                && (inner.dirty_projects.contains(&project_root)
                    || inner.dirty_projects.len() < WARM_DIRTY_PROJECT_CAPACITY)
            {
                inner.dirty_projects.insert(project_root);
            }
        }
    }

    fn wait_for_project_idle(&self, project_root: &Path, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut inner = self
            .inner
            .lock()
            .expect("Project cache warm state is not poisoned");
        loop {
            let busy = inner.latest.keys().any(|(root, _)| root == project_root)
                || inner.scheduled.iter().any(|(root, _)| root == project_root)
                || inner.dirty_projects.contains(project_root)
                || inner.maintenance_projects.contains(project_root);
            if !busy {
                return !inner.failures.keys().any(|(root, _)| root == project_root);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (next, wait) = self
                .idle
                .wait_timeout(inner, remaining)
                .expect("Project cache warm state is not poisoned");
            inner = next;
            if wait.timed_out() {
                return !inner.latest.keys().any(|(root, _)| root == project_root)
                    && !inner.scheduled.iter().any(|(root, _)| root == project_root)
                    && !inner.dirty_projects.contains(project_root)
                    && !inner.maintenance_projects.contains(project_root)
                    && !inner.failures.keys().any(|(root, _)| root == project_root);
            }
        }
    }
}

fn recover_building_scene_targets(project_root: &Path) -> Vec<super::WarmTarget> {
    use super::super::sync;
    let recovered = (|| -> anyhow::Result<Vec<super::WarmTarget>> {
        if sync::qualified_cache_root(project_root, false)?.is_none() { return Ok(Vec::new()); }
        sync::with_cache_lock(project_root, "recovery", || recover_managed_targets_locked(project_root))
    })();
    match recovered {
        Ok(targets) => targets,
        Err(error) => {
            log::warn!("cache recovery remains incomplete for {}: {error:#}", project_root.display());
            if let Err(barrier_error) = sync::set_cache_invalidation_barrier(project_root) {
                log::error!("could not preserve cache recovery barrier: {barrier_error:#}");
            }
            Vec::new()
        }
    }
}

fn recover_managed_targets_locked(project_root: &Path) -> anyhow::Result<Vec<super::WarmTarget>> {
    use super::super::sync::{self, PendingIntentStore, TombstoneStore};
    use super::super::identity::compute_scene_revision_identity;
    use anyhow::Context;
    let manifest = super::ManifestStore::read_validated(project_root)?;
    let scene_store = super::SceneCacheStore::new(project_root);
    let project_store = super::ProjectCacheStore::new(project_root);
    let pending = PendingIntentStore::new(project_root);
    let tombstones = TombstoneStore::new(project_root);
    // Validate the entire durable intent directory. A corrupt record must not
    // disappear from recovery merely because it was not enumerated.
    pending.list_all()?;
    let reconcile = sync::cache_invalidation_barrier(project_root)?;
    let default_config = super::super::compatibility::project_runtime_cache_config_hash(usd_semantic::SemanticConfig::default().hash());
    let mut targets = Vec::new();
    let mut registered = super::project_cache_targets(&manifest);
    if pending.load_record(&super::ProjectCacheTarget::ProjectRoot)?.is_some()
        && !registered.contains(&super::ProjectCacheTarget::ProjectRoot) {
        registered.push(super::ProjectCacheTarget::ProjectRoot);
    }
    for target in registered {
        let record = pending.load_record(&target)?;
        let config = default_config;
        let floor = tombstones.reactivation_floor(&target)?;
        let (actual, current, scene_id) = match &target {
            super::ProjectCacheTarget::Scene { id } => {
                let scene_id = usd_project::SceneId::parse(id)?;
                let descriptor = match scene_store.load_descriptor(scene_id) {
                    Ok(descriptor) => descriptor,
                    Err(_) if record.is_some() => None,
                    Err(error) => return Err(error),
                };
                (compute_scene_revision_identity(project_root, scene_id, RuntimeProfile::NativeMedium, config).map(|id| id.digest()), descriptor, Some(scene_id))
            }
            _ => (super::ProjectCacheIdentity::for_project(project_root, target.clone(), RuntimeProfile::NativeMedium, config)
                .and_then(|id| id.revision_digest(project_root)), None, None),
        };
        // Existing Building/Ready pointers without a durable record require
        // a fresh authorized generation. Do not warm unrelated missing targets.
        if !reconcile
            && record.is_none()
            && current.as_ref().is_none_or(|d| {
                !matches!(
                    d.state,
                    super::SceneCacheState::Building | super::SceneCacheState::Ready
                )
            })
        {
            continue;
        }
        let actual = match actual {
            Ok(digest) => Some(digest),
            Err(error) => {
                log::warn!("cannot qualify recovered target {}: {error:#}", target.key());
                None
            }
        };
        let needs_new = reconcile || record.as_ref().is_none_or(|record|
            record.generation <= floor || record.config_hash != config || record.desired_revision_identity != actual)
            || current.as_ref().is_some_and(|descriptor| record.as_ref().is_some_and(|record| descriptor.generation > record.generation));
        let generation = if needs_new {
            // Recompute and persist the desired state from today's canonical
            // identity. Never replay a stale identity with its old generation.
            match scene_id {
                Some(id) => scene_store.advance_generation(id, config)?,
                None => project_store.advance_generation(&target, config)?,
            }
        } else {
            let record = record.as_ref().context("missing durable desired generation")?;
            if !record.pending {
                // Acknowledged Scene descriptors may have become corrupt or
                // disappeared. Rebuild under a new generation in that case.
                let valid = match scene_id {
                    Some(id) => matches!(scene_store.load_activation(id), Ok(Some(_))),
                    None => super::ProjectCacheIdentity::for_project(project_root, target.clone(), RuntimeProfile::NativeMedium, config)
                        .and_then(|identity| project_store.load(&identity))
                        .is_ok_and(|descriptor| descriptor.is_some_and(|d| matches!(d.state, super::ProjectCacheState::Ready | super::ProjectCacheState::Empty))),
                };
                if valid { continue; }
                match scene_id {
                    Some(id) => scene_store.advance_generation(id, config)?,
                    None => project_store.advance_generation(&target, config)?,
                }
            } else {
                if let Some(id) = scene_id {
                    if current.as_ref().is_none_or(|d| d.generation != record.generation || d.state != super::SceneCacheState::Building) {
                        scene_store.publish_descriptor(&super::super::contract::SceneCacheDescriptorV3::invalidated(id, record.generation, record.config_hash))?;
                    }
                }
                record.generation
            }
        };
        // An unknown identity remains a durable pending obligation. Do not
        // hot-loop extraction while the source cannot be qualified.
        if pending.load_record(&target)?.is_some_and(|record| record.desired_revision_identity.is_some()) {
            targets.push(super::WarmTarget { target, scene_generation: Some(generation), build_generation: 0 });
        }
    }
    if reconcile { sync::clear_cache_invalidation_barrier(project_root)?; }
    targets.sort_by_key(|target| target.target.key());
    Ok(targets)
}

impl Default for ProjectCacheWarmQueue {
    fn default() -> Self {
        let (sender, receiver) = mpsc::sync_channel(WARM_QUEUE_CAPACITY);
        let sender = Arc::new(Mutex::new(Some(sender)));
        let latest = Arc::new(LatestWarmState::new());
        let worker_latest = Arc::clone(&latest);
        // Only queue owners keep the admission channel alive. The worker
        // borrows it for retry, never while waiting for the next job.
        let worker_sender = Arc::downgrade(&sender);
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

// No custom Drop: releasing the last state owner drops the channel sender
// and detaches the JoinHandle without locking or joining the worker.

impl ProjectCacheWarmQueue {
    pub(crate) fn shutdown_without_waiting(&self) {
        // Explicit lifecycle operation, not a destructor. Never join here.
        match self.state.sender.lock() {
            Ok(mut sender) => {
                sender.take();
            }
            Err(_) => {
                log::warn!("cache warm sender poisoned; final owner release will close admission")
            }
        }
        match self.state.worker.lock() {
            Ok(mut worker) => {
                worker.take();
            }
            Err(_) => log::warn!(
                "cache warm worker handle poisoned; final owner release will detach it"
            ),
        }
    }

    pub(super) fn cancel_target(&self, project_root: &Path, target: &super::ProjectCacheTarget) {
        self.state
            .latest
            .cancel(&(project_root.to_path_buf(), target.key()));
    }

    /// Request maintenance only after the canonical/retirement transaction has
    /// released its locks. Never make canonical success depend on queue admission.
    pub(crate) fn schedule_cache_maintenance(&self, project_root: &Path) {
        match super::super::sync::qualified_cache_root(project_root, false) {
            Ok(Some(_)) => {},
            Ok(None) => return,
            Err(error) => {
                log::warn!("cache maintenance refused for {}: {error:#}", project_root.display());
                return;
            }
        }
        let sender = self.state.sender.lock().ok().and_then(|sender| sender.clone());
        if !sender.as_ref().is_some_and(|sender| self.state.latest.request_maintenance(project_root, sender)) {
            log::debug!("cache maintenance deferred until later lifecycle/startup for {}", project_root.display());
        }
    }

    pub(crate) fn wait_for_project_idle(&self, project_root: &Path, timeout: Duration) -> bool {
        self.state
            .latest
            .wait_for_project_idle(project_root, timeout)
    }

    pub(crate) fn last_warm_failure(&self, project_root: &Path) -> Option<String> {
        self.state.latest.failure_for_project(project_root)
    }

    /// Re-admit durable pending preparation intents (for example after a crash
    /// or after an unavailable queue comes back online). Returns the number of
    /// recovered targets submitted to the worker.
    pub(crate) fn recover_pending_intents(&self, project_root: &Path) -> usize {
        let sender = match self.state.sender.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => {
                log::warn!(
                    "Project cache warm sender lock is poisoned; preserving durable pending intents for {}",
                    project_root.display()
                );
                return 0;
            }
        };
        let Some(sender) = sender else {
            return 0;
        };
        let recovered = recover_building_scene_targets(project_root);
        let mut admitted = 0;
        for mut warm_target in recovered {
            warm_target.build_generation = self
                .state
                .next_build_generation
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let key = (project_root.to_path_buf(), warm_target.target.key());
            if self.state.latest.record(key, warm_target, &sender) {
                admitted += 1;
            }
        }
        admitted
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
            super::super::compatibility::project_runtime_cache_config_hash(
                usd_semantic::SemanticConfig::default().hash(),
            ),
        ) {
            Ok(identity) => identity,
            Err(error) => {
                log::warn!(
                    "Project cache activation identity could not be established for {}: {error:#}",
                    project_root.display()
                );
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
                | super::ProjectCacheState::Partial
                | super::ProjectCacheState::Stale
                | super::ProjectCacheState::Failed
                | super::ProjectCacheState::Retired => ProjectCachePreparation::FallbackRequired,
            },
            Ok(None) => ProjectCachePreparation::FallbackRequired,
            Err(error) => {
                log::warn!(
                    "Project cache activation descriptor is unavailable for {}: {error:#}",
                    identity.target.key()
                );
                ProjectCachePreparation::FallbackRequired
            }
        }
    }
}

fn worker_loop(
    receiver: mpsc::Receiver<super::WarmJob>,
    sender: Weak<Mutex<Option<mpsc::SyncSender<super::WarmJob>>>>,
    latest: Arc<LatestWarmState>,
) {
    while let Ok(job) = receiver.recv() {
        // A queued job may outlive its final owner. Durable pending work is
        // recovered later; never keep this receiver alive on its own.
        if sender.strong_count() == 0 {
            break;
        }
        let mut request_maintenance = false;
        if job.key.1 == CACHE_MAINTENANCE_KEY {
            if latest.take_maintenance(&job.key.0) {
                run_cache_maintenance(&job.key.0);
            }
        } else if let Some(target) = latest.take_latest(&job.key) {
            if let Err(error) = super::warm_target(&job.key.0, &target) {
                let message = format!("{error:#}");
                latest.record_failure(&job.key, message.clone());
                super::mark_warm_failure_terminal(&job.key.0, &target);
                log::warn!(
                    "Project cache warm failed for {} ({}): {message}",
                    job.key.0.display(),
                    target.target.key()
                );
            } else {
                // The build's source/pin/Scene guards have been dropped.
                request_maintenance = true;
            }
        }
        let Some(owner) = sender.upgrade() else {
            latest.finish_and_retry(&job.key, None);
            break;
        };
        let available = match owner.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => {
                log::warn!("cache warm sender poisoned; preserving durable pending work");
                latest.finish_and_retry(&job.key, None);
                break;
            }
        };
        drop(owner);
        let Some(sender) = available.as_ref() else {
            latest.finish_and_retry(&job.key, None);
            break;
        };
        if request_maintenance {
            if !latest.request_maintenance(&job.key.0, sender) {
                log::debug!("cache maintenance deferred for {}", job.key.0.display());
            }
        }
        latest.finish_and_retry(&job.key, Some(sender));
        // `available` drops before recv(), so the worker never owns an
        // enduring sender that could prevent channel disconnect.
    }
}

fn run_cache_maintenance(project_root: &Path) {
    // The GC owns its Project transaction. No queue or Scene lifecycle mutex
    // is held here, and a missing/invalid Project never enables collection.
    if let Err(error) = super::ManifestStore::read_validated(project_root) {
        log::warn!("cache maintenance deferred for {}: {error:#}", project_root.display());
        return;
    }
    match super::super::gc::collect_garbage(project_root, super::super::gc::GcPolicy::default()) {
        Ok(report) if report.aborted_reason.is_some() => {
            log::warn!("cache maintenance deferred for {}: {}", project_root.display(), report.aborted_reason.as_deref().unwrap_or("uncertain reachability"));
        }
        Ok(report) => log::debug!("cache maintenance {}: {} revisions, {} blobs, {} staging dirs removed",
            project_root.display(), report.removed_revisions, report.removed_blobs, report.removed_temps),
        Err(error) => log::warn!("cache maintenance failed for {}: {error:#}", project_root.display()),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/project/cache/warming/queue_test_support.rs"]
pub(super) mod test_support;
