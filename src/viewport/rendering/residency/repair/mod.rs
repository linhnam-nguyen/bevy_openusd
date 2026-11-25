//! Bounded, keyed repair admission and off-thread cache persistence.

use std::path::PathBuf;

use bevy::prelude::{Commands, NonSend, Res, ResMut};

use crate::cache::demand::{
    OwnedPrimPathResolution, TargetedSceneRepair, extract_scene_payloads_for_repair,
    owned_prim_path_for_payload,
};
use crate::cache::hydration::ActiveProjectCacheContext;
use crate::viewport::stage::SceneCacheOwnershipContext;

use super::active_cache_project_root;
use super::authority::{ResidencyAuthority, ScenePayloadKey};
use super::loader::LoadJob;
use super::worker::{
    CachedResidencyWorker, LoadedScenePayloadQueue, WorkerCompletion, queue_loaded_scene_payloads,
    reject_scene_payload,
};

mod handoff;
pub(crate) mod persistence;
pub(crate) mod phase;

use handoff::enqueue_repair_or_requeue;
pub(crate) use handoff::{TargetedRepairQueue, drain_targeted_repair_persistence_completions};
pub(crate) use persistence::{TargetedRepairPersistenceRequest, TargetedRepairPersistenceWorker};
pub(crate) use phase::RepairPhase;

pub(crate) const TARGETED_REPAIR_QUEUE_CAPACITY: usize = 8;
pub(crate) const TARGETED_REPAIRS_PER_UPDATE: usize = 1;
pub(crate) const TARGETED_REPAIR_PERSISTENCE_QUEUE_CAPACITY: usize = 2;

#[derive(Debug)]
pub(crate) struct TargetedRepairRequest {
    pub(crate) project_root: Option<PathBuf>,
    pub(crate) path: Option<String>,
    pub(crate) extraction: Option<TargetedSceneRepair>,
    pub(crate) lookup_only: bool,
    pub(crate) phase: RepairPhase,
    pub(crate) job: LoadJob<ScenePayloadKey>,
}

pub(crate) fn drain_cached_residency_completions(
    scene_owner: Option<Res<SceneCacheOwnershipContext>>,
    legacy_cache: Option<Res<ActiveProjectCacheContext>>,
    worker: Res<CachedResidencyWorker>,
    mut authority: ResMut<ResidencyAuthority>,
    mut repairs: ResMut<TargetedRepairQueue>,
    mut payload_queue: Option<ResMut<LoadedScenePayloadQueue>>,
    mut commands: Commands,
) {
    let project_root = active_cache_project_root(scene_owner.as_deref(), legacy_cache.as_deref());
    for completion in worker.drain_completions() {
        let completion = match completion {
            WorkerCompletion::Cached(completion) => completion,
            WorkerCompletion::Source { identity, mesh, .. } => {
                commands.queue(move |world: &mut bevy::prelude::World| {
                    super::complete_source_mesh(world, identity, mesh)
                });
                continue;
            }
        };
        if !authority.load_is_current(&completion.job) {
            continue;
        }
        match completion.result {
            Ok(mesh) => {
                if mesh.is_none() && completion.mask.geometry {
                    enqueue_missing_payload_repair(
                        project_root.as_deref(),
                        &mut authority,
                        &mut repairs,
                        completion.job,
                    );
                    continue;
                }
                let needs_installation = completion.payloads.as_ref().is_some_and(|payloads| {
                    !payloads.materials.is_empty()
                        || !payloads.textures.is_empty()
                        || !payloads.animations.is_empty()
                });
                if needs_installation {
                    // Do not acknowledge material/animation or geometry while
                    // the device-specific payload conversion is still pending.
                    if let (Some(payloads), Some(queue)) =
                        (completion.payloads, payload_queue.as_deref_mut())
                    {
                        if let Err(job) = queue_loaded_scene_payloads(
                            queue,
                            completion.job,
                            completion.mask,
                            mesh,
                            payloads,
                        ) {
                            let _ = authority.defer_load(job);
                        }
                    } else {
                        reject_scene_payload(
                            &mut authority,
                            None,
                            &completion.job,
                            "required Scene payload installation queue unavailable",
                        );
                    }
                } else if let Some(mesh) = mesh {
                    authority.clear_repair_phase(&completion.job);
                    let _ = authority.complete_cached_cpu_with_mask(
                        completion.job,
                        mesh,
                        completion.mask,
                    );
                } else {
                    let _ = authority.complete_cached_payloads(&completion.job, completion.mask);
                }
            }
            Err(error) => {
                reject_scene_payload(&mut authority, None, &completion.job, &error);
            }
        }
    }
}

fn enqueue_missing_payload_repair(
    project_root: Option<&std::path::Path>,
    authority: &mut ResidencyAuthority,
    repairs: &mut TargetedRepairQueue,
    job: LoadJob<ScenePayloadKey>,
) {
    if !authority.wait_for_repair(&job) {
        return;
    }
    let phase = authority.take_repair_phase(&job);
    let request = if phase.is_scene_payload() {
        let (project_root, path) = match project_root {
            Some(project_root) => {
                match owned_prim_path_for_payload(project_root, job.key.scene_id, job.key.blob_hash)
                {
                    Ok(OwnedPrimPathResolution::Ready(path)) => {
                        (Some(project_root.to_path_buf()), Some(path))
                    }
                    Ok(OwnedPrimPathResolution::Waiting) => {
                        (Some(project_root.to_path_buf()), None)
                    }
                    Ok(OwnedPrimPathResolution::Rejected) => {
                        let _ = authority.suppress_repair(&job);
                        return;
                    }
                    Err(_) => {
                        let _ = authority.reject_repair(&job);
                        return;
                    }
                }
            }
            None => (None, None),
        };
        TargetedRepairRequest {
            project_root,
            path,
            extraction: None,
            lookup_only: false,
            phase,
            job: job.clone(),
        }
    } else {
        let Some((phase_root, phase_path, expected_descriptor)) = phase
            .lookup_parts()
            .map(|(root, path, descriptor)| (root.clone(), path.to_owned(), descriptor.clone()))
        else {
            let _ = authority.reject_repair(&job);
            return;
        };
        if project_root.is_some_and(|root| root != phase_root.as_path()) {
            let _ = authority.reject_repair(&job);
            return;
        }
        let phase = match phase {
            RepairPhase::CacheReady {
                project_root,
                path,
                expected_descriptor,
            } => RepairPhase::NeedProjectLookup {
                project_root,
                path,
                expected_descriptor,
            },
            phase => phase,
        };
        TargetedRepairRequest {
            project_root: Some(phase_root),
            path: Some(phase_path),
            extraction: Some(TargetedSceneRepair {
                expected_descriptor,
                payloads: Vec::new(),
            }),
            lookup_only: true,
            phase,
            job: job.clone(),
        }
    };
    enqueue_repair_or_requeue(authority, repairs, request);
}

pub(crate) fn process_targeted_residency_repairs(
    scene_owner: Option<Res<SceneCacheOwnershipContext>>,
    legacy_cache: Option<Res<ActiveProjectCacheContext>>,
    persistence: Res<TargetedRepairPersistenceWorker>,
    mut authority: ResMut<ResidencyAuthority>,
    mut repairs: ResMut<TargetedRepairQueue>,
    live: Option<NonSend<usd_bevy::LiveStage>>,
) {
    let active_root = active_cache_project_root(scene_owner.as_deref(), legacy_cache.as_deref());
    let mut attempts = repairs.len();
    let mut dispatched = 0;
    while attempts > 0 {
        attempts -= 1;
        let Some(mut request) = repairs.take_front() else {
            break;
        };
        if !authority.repair_waiting_is_current(&request.job) {
            continue;
        }
        let Some(project_root) = active_root.as_deref() else {
            repairs.push_back(request);
            break;
        };
        let project_root = project_root.to_path_buf();
        if request.project_root.as_ref() != Some(&project_root) {
            if !request.phase.is_scene_payload() {
                let _ = authority.reject_repair(&request.job);
                continue;
            }
            request.project_root = Some(project_root.clone());
            request.path = None;
            request.extraction = None;
            request.lookup_only = false;
        }
        if request.lookup_only {
            let Some(extraction) = request.extraction.take() else {
                repairs.push_back(request);
                continue;
            };
            let persistence_request = TargetedRepairPersistenceRequest {
                project_root,
                path: request.path.unwrap_or_default(),
                extraction,
                lookup_only: true,
                phase: request.phase,
                job: request.job,
            };
            if let Err(persistence_request) = persistence.dispatch(persistence_request) {
                repairs.push_back(persistence_request.into_repair_request());
                break;
            }
            dispatched += 1;
            if dispatched >= TARGETED_REPAIRS_PER_UPDATE {
                break;
            }
            continue;
        }
        match owned_prim_path_for_payload(
            &project_root,
            request.job.key.scene_id,
            request.job.key.blob_hash,
        ) {
            Ok(OwnedPrimPathResolution::Ready(path)) => {
                if request.path.as_deref() != Some(path.as_str()) {
                    request.path = Some(path);
                    request.extraction = None;
                }
            }
            Ok(OwnedPrimPathResolution::Waiting) => {
                repairs.push_back(request);
                continue;
            }
            Ok(OwnedPrimPathResolution::Rejected) => {
                let _ = authority.suppress_repair(&request.job);
                continue;
            }
            Err(_) => {
                let _ = authority.reject_repair(&request.job);
                continue;
            }
        }
        let Some(path) = request.path.clone() else {
            repairs.push_back(request);
            continue;
        };
        if request.extraction.is_none() {
            let Some(live) = live.as_deref() else {
                repairs.push_back(request);
                continue;
            };
            match extract_scene_payloads_for_repair(
                &project_root,
                request.job.key.scene_id,
                request.job.key.blob_hash,
                live,
                &path,
            ) {
                Ok(Some(extraction)) => request.extraction = Some(extraction),
                Ok(None) => {
                    repairs.push_back(request);
                    continue;
                }
                Err(_) => {
                    let _ = authority.reject_repair(&request.job);
                    continue;
                }
            }
        }
        let Some(extraction) = request.extraction.take() else {
            repairs.push_back(request);
            break;
        };
        let persistence_request = TargetedRepairPersistenceRequest {
            project_root,
            path,
            extraction,
            lookup_only: request.lookup_only,
            phase: request.phase,
            job: request.job,
        };
        if let Err(persistence_request) = persistence.dispatch(persistence_request) {
            repairs.push_back(persistence_request.into_repair_request());
            break;
        }
        dispatched += 1;
        if dispatched >= TARGETED_REPAIRS_PER_UPDATE {
            break;
        }
    }
}

#[cfg(test)]
#[path = "../../../../../tests/unit/viewport/residency/repair_integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "../../../../../tests/unit/viewport/residency/repair_persistence_tests.rs"]
mod persistence_tests;
#[cfg(test)]
#[path = "../../../../../tests/unit/viewport/residency/repair_pressure_tests.rs"]
mod pressure_tests;
#[cfg(test)]
#[path = "../../../../../tests/unit/viewport/residency/repair_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "../../../../../tests/unit/viewport/residency/repair_waiting_tests.rs"]
mod waiting_tests;
