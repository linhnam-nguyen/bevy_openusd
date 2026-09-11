use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use bevy::prelude::{Res, ResMut, Resource};

use crate::project::cache_contract::SceneCacheDescriptorV3;
use crate::project::cache_demand_projection::{TargetedPersistenceOutcome, TargetedSceneRepair};

use super::super::authority::{ResidencyAuthority, ScenePayloadKey};
use super::super::loader::LoadJob;
use super::super::repair_phase::RepairPhase;
use super::super::repair_persistence::{
    TargetedRepairPersistenceCompletion, TargetedRepairPersistenceWorker,
};
use super::{TargetedRepairRequest, TARGETED_REPAIR_QUEUE_CAPACITY};

#[derive(Debug, Resource, Default)]
pub(crate) struct TargetedRepairQueue {
    order: VecDeque<(ScenePayloadKey, u64)>,
    requests: HashMap<(ScenePayloadKey, u64), TargetedRepairRequest>,
}

impl TargetedRepairQueue {
    pub(crate) fn enqueue(&mut self, request: TargetedRepairRequest) -> bool {
        let identity = (request.job.key, request.job.generation);
        if self.requests.contains_key(&identity) {
            self.requests.insert(identity, request);
            return true;
        }
        if self.requests.len() >= TARGETED_REPAIR_QUEUE_CAPACITY {
            return false;
        }
        self.order.push_back(identity);
        self.requests.insert(identity, request);
        true
    }

    pub(super) fn take_front(&mut self) -> Option<TargetedRepairRequest> {
        while let Some(identity) = self.order.pop_front() {
            if let Some(request) = self.requests.remove(&identity) {
                return Some(request);
            }
        }
        None
    }

    pub(super) fn push_back(&mut self, request: TargetedRepairRequest) {
        let identity = (request.job.key, request.job.generation);
        self.requests.insert(identity, request);
        self.order.retain(|queued| queued != &identity);
        self.order.push_back(identity);
    }

    pub(crate) fn clear(&mut self) {
        self.order.clear();
        self.requests.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.requests.len()
    }
}

pub(super) fn enqueue_repair_or_requeue(
    authority: &mut ResidencyAuthority,
    repairs: &mut TargetedRepairQueue,
    request: TargetedRepairRequest,
) {
    let job = request.job.clone();
    let phase = request.phase.clone();
    if repairs.enqueue(request) || authority.requeue_repair_waiting(&job, phase) {
        return;
    }
    let _ = authority.reject_repair(&job);
}

pub(super) fn enqueue_scene_payload_repair(
    authority: &mut ResidencyAuthority,
    repairs: &mut TargetedRepairQueue,
    job: LoadJob<ScenePayloadKey>,
    project_root: PathBuf,
    path: String,
) {
    enqueue_repair_or_requeue(
        authority,
        repairs,
        TargetedRepairRequest {
            project_root: Some(project_root),
            path: Some(path),
            extraction: None,
            lookup_only: false,
            phase: RepairPhase::NeedScenePayload,
            job,
        },
    );
}

pub(super) fn enqueue_lookup_repair(
    authority: &mut ResidencyAuthority,
    repairs: &mut TargetedRepairQueue,
    job: LoadJob<ScenePayloadKey>,
    project_root: PathBuf,
    path: String,
    expected_descriptor: SceneCacheDescriptorV3,
) {
    let phase = RepairPhase::NeedProjectLookup {
        project_root: project_root.clone(),
        path: path.clone(),
        expected_descriptor: expected_descriptor.clone(),
    };
    enqueue_repair_or_requeue(
        authority,
        repairs,
        TargetedRepairRequest {
            project_root: Some(project_root),
            path: Some(path),
            extraction: Some(TargetedSceneRepair {
                expected_descriptor,
                payloads: Vec::new(),
            }),
            lookup_only: true,
            phase,
            job,
        },
    );
}

pub(super) fn retry_cached_load(
    authority: &mut ResidencyAuthority,
    repairs: &mut TargetedRepairQueue,
    job: LoadJob<ScenePayloadKey>,
    project_root: PathBuf,
    path: String,
    expected_descriptor: SceneCacheDescriptorV3,
) {
    let phase = RepairPhase::CacheReady {
        project_root: project_root.clone(),
        path: path.clone(),
        expected_descriptor: expected_descriptor.clone(),
    };
    if authority.requeue_repair_waiting(&job, phase.clone()) {
        return;
    }
    enqueue_repair_or_requeue(
        authority,
        repairs,
        TargetedRepairRequest {
            project_root: Some(project_root),
            path: Some(path),
            extraction: Some(TargetedSceneRepair {
                expected_descriptor,
                payloads: Vec::new(),
            }),
            lookup_only: true,
            phase,
            job,
        },
    );
}

pub(crate) fn drain_targeted_repair_persistence_completions(
    worker: Res<TargetedRepairPersistenceWorker>,
    mut authority: ResMut<ResidencyAuthority>,
    mut repairs: ResMut<TargetedRepairQueue>,
) {
    for completion in worker.drain_completions() {
        if !authority.repair_waiting_is_current(&completion.job) {
            continue;
        }
        let TargetedRepairPersistenceCompletion {
            job,
            project_root,
            path,
            lookup_only,
            phase,
            result,
        } = completion;
        let lookup_descriptor = phase
            .lookup_parts()
            .map(|(_, _, descriptor)| descriptor.clone());
        match result {
            Ok(TargetedPersistenceOutcome::Published {
                lookup_repaired: true,
                published_descriptor,
            }) => retry_cached_load(
                &mut authority,
                &mut repairs,
                job,
                project_root,
                path,
                published_descriptor,
            ),
            Ok(TargetedPersistenceOutcome::LookupRepaired) => {
                let Some(expected_descriptor) = lookup_descriptor else {
                    let _ = authority.reject_repair(&job);
                    continue;
                };
                retry_cached_load(
                    &mut authority,
                    &mut repairs,
                    job,
                    project_root,
                    path,
                    expected_descriptor,
                );
            }
            Ok(TargetedPersistenceOutcome::Published {
                lookup_repaired: false,
                published_descriptor,
            }) => enqueue_lookup_repair(
                &mut authority,
                &mut repairs,
                job,
                project_root,
                path,
                published_descriptor,
            ),
            Ok(TargetedPersistenceOutcome::LookupPending) => {
                let Some(expected_descriptor) = lookup_descriptor else {
                    let _ = authority.reject_repair(&job);
                    continue;
                };
                enqueue_lookup_repair(
                    &mut authority,
                    &mut repairs,
                    job,
                    project_root,
                    path,
                    expected_descriptor,
                );
            }
            Ok(TargetedPersistenceOutcome::CasLost | TargetedPersistenceOutcome::ScenePayloadMissing) =>
                enqueue_scene_payload_repair(
                    &mut authority,
                    &mut repairs,
                    job,
                    project_root,
                    path,
                ),
            Ok(TargetedPersistenceOutcome::ScenePayloadNotOwned) => {
                let _ = authority.suppress_repair(&job);
            }
            Ok(TargetedPersistenceOutcome::Waiting) => {
                let (extraction, lookup_only, phase) = if lookup_only {
                    let Some(expected_descriptor) = lookup_descriptor else {
                        let _ = authority.reject_repair(&job);
                        continue;
                    };
                    (
                        Some(TargetedSceneRepair {
                            expected_descriptor,
                            payloads: Vec::new(),
                        }),
                        true,
                        phase,
                    )
                } else {
                    (None, false, RepairPhase::NeedScenePayload)
                };
                enqueue_repair_or_requeue(
                    &mut authority,
                    &mut repairs,
                    TargetedRepairRequest {
                        project_root: Some(project_root),
                        path: Some(path),
                        extraction,
                        lookup_only,
                        phase,
                        job,
                    },
                );
            }
            Err(_) => {
                let _ = authority.reject_repair(&job);
            }
        }
    }
}
