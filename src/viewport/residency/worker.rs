//! Bounded cache read/decode work for viewport residency.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use bevy::mesh::Mesh;
use usd_model::BlobId;

use bevy::prelude::Resource;

use crate::project::blob_store::get_mesh;
use crate::project::cache::SceneCacheStore;

use super::ScenePayloadKey;
use super::loader::LoadJob;

pub(crate) const WORKER_QUEUE_CAPACITY: usize = 8;

struct LoadRequest {
    project_root: PathBuf,
    job: LoadJob<ScenePayloadKey>,
}

pub(crate) struct LoadCompletion {
    pub(crate) job: LoadJob<ScenePayloadKey>,
    pub(crate) result: Result<Option<Mesh>, String>,
}

#[derive(Resource)]
pub(crate) struct CachedResidencyWorker {
    requests: Option<SyncSender<LoadRequest>>,
    completions: Option<Mutex<Receiver<LoadCompletion>>>,
    thread: Option<JoinHandle<()>>,
    available: AtomicBool,
}

impl Default for CachedResidencyWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl CachedResidencyWorker {
    pub(crate) fn new() -> Self {
        let (request_tx, request_rx) = mpsc::sync_channel::<LoadRequest>(WORKER_QUEUE_CAPACITY);
        let (completion_tx, completion_rx) =
            mpsc::sync_channel::<LoadCompletion>(WORKER_QUEUE_CAPACITY);
        let thread = thread::Builder::new()
            .name("viewport-residency-cache".to_owned())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    let result = load_cached_mesh(&request.project_root, &request.job);
                    if completion_tx
                        .send(LoadCompletion {
                            job: request.job,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
        match thread {
            Ok(thread) => Self {
                requests: Some(request_tx),
                completions: Some(Mutex::new(completion_rx)),
                thread: Some(thread),
                available: AtomicBool::new(true),
            },
            Err(error) => {
                bevy::log::error!(
                    "[viewport-residency] cached worker unavailable; cached demand remains deferred: {error}"
                );
                Self {
                    requests: None,
                    completions: Some(Mutex::new(completion_rx)),
                    thread: None,
                    available: AtomicBool::new(false),
                }
            }
        }
    }

    pub(crate) fn is_available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn unavailable_for_test() -> Self {
        Self {
            requests: None,
            completions: None,
            thread: None,
            available: AtomicBool::new(false),
        }
    }

    pub(crate) fn dispatch(
        &self,
        project_root: PathBuf,
        job: LoadJob<ScenePayloadKey>,
    ) -> Result<(), LoadJob<ScenePayloadKey>> {
        if !self.is_available() {
            return Err(job);
        }
        let Some(requests) = self.requests.as_ref() else {
            return Err(job);
        };
        let request = LoadRequest { project_root, job };
        match requests.try_send(request) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(request)) => Err(request.job),
            Err(TrySendError::Disconnected(request)) => {
                self.available.store(false, Ordering::Release);
                Err(request.job)
            }
        }
    }

    pub(crate) fn drain_completions(&self) -> Vec<LoadCompletion> {
        let Some(completions) = self.completions.as_ref() else {
            return Vec::new();
        };
        let Ok(receiver) = completions.lock() else {
            return Vec::new();
        };
        let mut completions = Vec::with_capacity(WORKER_QUEUE_CAPACITY);
        while completions.len() < WORKER_QUEUE_CAPACITY {
            match receiver.try_recv() {
                Ok(completion) => completions.push(completion),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        completions
    }
}

impl Drop for CachedResidencyWorker {
    fn drop(&mut self) {
        self.available.store(false, Ordering::Release);
        self.requests.take();
        self.completions.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn load_cached_mesh(
    project_root: &std::path::Path,
    job: &LoadJob<ScenePayloadKey>,
) -> Result<Option<Mesh>, String> {
    let store = SceneCacheStore::new(project_root)
        .object_store(job.key.scene_id)
        .map_err(|error| error.to_string())?;
    get_mesh(&store, &BlobId(job.key.blob_hash.to_string())).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use usd_model::HashDigest;
    use usd_project::SceneId;

    fn job(generation: u64) -> LoadJob<ScenePayloadKey> {
        LoadJob {
            key: ScenePayloadKey {
                scene_id: SceneId::new_v4(),
                blob_hash: HashDigest::new([7; HashDigest::BYTE_LEN]),
            },
            generation,
            cpu_bytes: 1,
            gpu_bytes: 1,
        }
    }

    #[test]
    fn worker_completion_keeps_generation_tagged_and_bounded() {
        let worker = CachedResidencyWorker::new();
        let job = job(17);
        let root = tempfile::tempdir().expect("temporary cache root");
        worker
            .dispatch(root.path().to_path_buf(), job.clone())
            .expect("bounded worker accepts first request");
        let completion = (0..100).find_map(|_| {
            let completion = worker.drain_completions().into_iter().next();
            if completion.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            completion
        });
        let completion = completion.expect("worker returns bounded completion");
        assert_eq!(completion.job.generation, job.generation);
        assert!(
            completion
                .result
                .expect("missing cache is not a worker error")
                .is_none()
        );
    }

    #[test]
    fn unavailable_worker_defers_without_publishing_completion() {
        let worker = CachedResidencyWorker::unavailable_for_test();
        let job = job(23);
        let root = tempfile::tempdir().expect("temporary cache root");
        let returned = worker
            .dispatch(root.path().to_path_buf(), job.clone())
            .expect_err("unavailable worker must defer demand");
        assert_eq!(returned, job);
        assert!(worker.drain_completions().is_empty());
    }

    #[test]
    fn disconnected_worker_becomes_unavailable_instead_of_requeueing_forever() {
        let (request_tx, request_rx) =
            mpsc::sync_channel::<LoadRequest>(WORKER_QUEUE_CAPACITY);
        drop(request_rx);
        let worker = CachedResidencyWorker {
            requests: Some(request_tx),
            completions: None,
            thread: None,
            available: std::sync::atomic::AtomicBool::new(true),
        };
        let root = tempfile::tempdir().expect("temporary cache root");
        assert!(worker.dispatch(root.path().to_path_buf(), job(29)).is_err());
        assert!(!worker.is_available());
    }
}
