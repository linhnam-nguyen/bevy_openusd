//! Bounded residency-owned persistence for targeted Scene repairs.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};

use bevy::prelude::Resource;

use crate::project::cache_demand_projection::{
    TargetedPersistenceOutcome, TargetedSceneRepair, persist_lookup_repair,
    persist_scene_payloads_for_repair,
};

use super::authority::ScenePayloadKey;
use super::loader::LoadJob;
use super::repair_phase::RepairPhase;
use super::repair::TargetedRepairRequest;
use super::repair::TARGETED_REPAIR_PERSISTENCE_QUEUE_CAPACITY as QUEUE_CAPACITY;

#[derive(Debug)]
pub(crate) struct TargetedRepairPersistenceRequest {
    pub(crate) project_root: PathBuf,
    pub(crate) path: String,
    pub(crate) extraction: TargetedSceneRepair,
    pub(crate) lookup_only: bool,
    pub(crate) phase: RepairPhase,
    pub(crate) job: LoadJob<ScenePayloadKey>,
}

impl TargetedRepairPersistenceRequest {
    pub(crate) fn into_repair_request(self) -> TargetedRepairRequest {
        TargetedRepairRequest {
            project_root: Some(self.project_root),
            path: Some(self.path),
            extraction: Some(self.extraction),
            lookup_only: self.lookup_only,
            phase: self.phase,
            job: self.job,
        }
    }
}

pub(crate) struct TargetedRepairPersistenceCompletion {
    pub(crate) job: LoadJob<ScenePayloadKey>,
    pub(crate) project_root: PathBuf,
    pub(crate) path: String,
    pub(crate) lookup_only: bool,
    pub(crate) phase: RepairPhase,
    pub(crate) result: Result<TargetedPersistenceOutcome, String>,
}

#[derive(Resource)]
pub(crate) struct TargetedRepairPersistenceWorker {
    requests: Option<SyncSender<TargetedRepairPersistenceRequest>>,
    completions: Option<Mutex<Receiver<TargetedRepairPersistenceCompletion>>>,
    thread: Option<JoinHandle<()>>,
    available: AtomicBool,
}

impl Default for TargetedRepairPersistenceWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl TargetedRepairPersistenceWorker {
    pub(crate) fn new() -> Self {
        let (request_tx, request_rx) =
            mpsc::sync_channel::<TargetedRepairPersistenceRequest>(QUEUE_CAPACITY);
        let (completion_tx, completion_rx) =
            mpsc::sync_channel::<TargetedRepairPersistenceCompletion>(QUEUE_CAPACITY);
        let thread = thread::Builder::new()
            .name("viewport-residency-repair".to_owned())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    let expected_descriptor = request.extraction.expected_descriptor.clone();
                    let result = if request.lookup_only {
                        persist_lookup_repair(
                            &request.project_root,
                            request.job.key.scene_id,
                            &expected_descriptor,
                            &request.path,
                            request.job.key.blob_hash,
                        )
                    } else {
                        persist_scene_payloads_for_repair(
                            &request.project_root,
                            request.job.key.scene_id,
                            &expected_descriptor,
                            request.extraction.payloads,
                        )
                    };
                    let result = result.map_err(|error| error.to_string());
                    if completion_tx
                        .send(TargetedRepairPersistenceCompletion {
                            job: request.job,
                            project_root: request.project_root,
                            path: request.path,
                            lookup_only: request.lookup_only,
                            phase: request.phase,
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
                    "[viewport-residency] targeted repair worker unavailable: {error}"
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

    pub(crate) fn dispatch(
        &self,
        request: TargetedRepairPersistenceRequest,
    ) -> Result<(), TargetedRepairPersistenceRequest> {
        if !self.available.load(Ordering::Acquire) {
            return Err(request);
        }
        let Some(requests) = self.requests.as_ref() else {
            return Err(request);
        };
        match requests.try_send(request) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(request)) => Err(request),
            Err(TrySendError::Disconnected(request)) => {
                self.available.store(false, Ordering::Release);
                Err(request)
            }
        }
    }

    pub(crate) fn drain_completions(&self) -> Vec<TargetedRepairPersistenceCompletion> {
        let Some(completions) = self.completions.as_ref() else {
            return Vec::new();
        };
        let Ok(receiver) = completions.lock() else {
            return Vec::new();
        };
        let mut drained = Vec::with_capacity(QUEUE_CAPACITY);
        while drained.len() < QUEUE_CAPACITY {
            match receiver.try_recv() {
                Ok(completion) => drained.push(completion),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        drained
    }

    #[cfg(test)]
    pub(crate) fn with_completion_for_test(
        completion: TargetedRepairPersistenceCompletion,
    ) -> Self {
        let (completion_tx, completion_rx) = mpsc::sync_channel(QUEUE_CAPACITY);
        completion_tx
            .send(completion)
            .expect("test repair completion queue accepts one completion");
        drop(completion_tx);
        Self {
            requests: None,
            completions: Some(Mutex::new(completion_rx)),
            thread: None,
            available: AtomicBool::new(false),
        }
    }
}

impl Drop for TargetedRepairPersistenceWorker {
    fn drop(&mut self) {
        self.available.store(false, Ordering::Release);
        self.requests.take();
        self.completions.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
