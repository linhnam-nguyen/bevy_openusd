//! Bounded cache read/decode work for viewport residency.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use bevy::mesh::Mesh;
use usd_bevy::live::source_mesh::{SourceMeshIdentity, SourceMeshInput};
use usd_bevy::route::material::UsdTextureCache;
use usd_model::BlobId;

use bevy::prelude::Resource;

use crate::cache::SceneCacheStore;
use crate::cache::blobs::get_mesh;
use crate::cache::contract::SceneCacheAddress;
use crate::cache::scene::hydration::SceneAnimationPayloads;
use crate::cache::scene::payload::SceneAnimationBlob;
use crate::document::runtime::payload::{RuntimeMaterialBlob, RuntimeTexturePayload};

use super::loader::LoadJob;
use super::visual_composition::VisualStreamingHealth;
use super::{PayloadLoadMask, ResidencyAuthority, ScenePayloadDescriptor, ScenePayloadKey};

mod payloads;
use payloads::load_cached_scene_payloads;

pub(crate) const WORKER_QUEUE_CAPACITY: usize = 8;
const WORKER_THREADS: usize = 4;
const WORKER_INPUT_BUDGET: u64 = 16 * 1024 * 1024;

struct LoadRequest {
    project_root: PathBuf,
    job: LoadJob<ScenePayloadKey>,
    mask: PayloadLoadMask,
    descriptors: Vec<ScenePayloadDescriptor>,
}

pub(crate) struct LoadCompletion {
    pub(crate) job: LoadJob<ScenePayloadKey>,
    pub(crate) mask: PayloadLoadMask,
    pub(crate) result: Result<Option<Mesh>, String>,
    pub(crate) payloads: Option<LoadedScenePayloads>,
}

pub(crate) struct LoadedScenePayloads {
    pub(crate) materials: Vec<(String, RuntimeMaterialBlob)>,
    pub(crate) textures: Vec<(String, RuntimeTexturePayload)>,
    pub(crate) animations: Vec<(SceneCacheAddress, SceneAnimationBlob)>,
}

#[derive(Resource, Default)]
pub(crate) struct LoadedScenePayloadQueue {
    completions: VecDeque<LoadedScenePayloadCompletion>,
}

struct LoadedScenePayloadCompletion {
    job: LoadJob<ScenePayloadKey>,
    mask: PayloadLoadMask,
    mesh: Option<Mesh>,
    payloads: LoadedScenePayloads,
}

enum WorkerRequest {
    Cached(LoadRequest),
    Source(SourceMeshInput),
}

pub(crate) enum WorkerCompletion {
    Cached(LoadCompletion),
    Source {
        identity: SourceMeshIdentity,
        bytes: u64,
        mesh: Option<Mesh>,
    },
}

impl WorkerRequest {
    fn bytes(&self) -> u64 {
        match self {
            Self::Cached(request) => request.job.cpu_bytes,
            Self::Source(input) => input.estimated_bytes(),
        }
    }

    fn prepare(self) -> WorkerCompletion {
        match self {
            Self::Cached(request) => WorkerCompletion::Cached(prepare_cached_request(request)),
            Self::Source(input) => {
                let bytes = input.estimated_bytes();
                let identity = input.identity;
                let mesh =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| input.prepare())).ok();
                WorkerCompletion::Source {
                    identity,
                    bytes,
                    mesh,
                }
            }
        }
    }
}

type SequencedRequest = (u64, WorkerRequest);
type SequencedCompletion = (u64, WorkerCompletion);

#[derive(Default)]
struct WorkerWindow {
    next_request: u64,
    next_publish: u64,
    reserved_bytes: u64,
    ready: BTreeMap<u64, WorkerCompletion>,
}

pub(super) struct SourceWorkCapacity {
    pub(super) jobs: usize,
    pub(super) bytes: u64,
    pub(super) allow_oversized: bool,
}

#[derive(Resource)]
pub(crate) struct CachedResidencyWorker {
    requests: Option<SyncSender<SequencedRequest>>,
    completions: Option<Mutex<Receiver<SequencedCompletion>>>,
    threads: Vec<JoinHandle<()>>,
    window: Mutex<WorkerWindow>,
    available: AtomicBool,
}

impl Default for CachedResidencyWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl CachedResidencyWorker {
    pub(crate) fn new() -> Self {
        let (request_tx, request_rx) =
            mpsc::sync_channel::<SequencedRequest>(WORKER_QUEUE_CAPACITY);
        let requests = Arc::new(Mutex::new(request_rx));
        let (completion_tx, completion_rx) =
            mpsc::sync_channel::<SequencedCompletion>(WORKER_QUEUE_CAPACITY);
        let mut threads = Vec::with_capacity(WORKER_THREADS);
        for slot in 0..WORKER_THREADS {
            let requests = Arc::clone(&requests);
            let completions = completion_tx.clone();
            let worker = thread::Builder::new()
                .name(format!("viewport-residency-cache-{slot}"))
                .spawn(move || {
                    while let Ok((sequence, request)) = receive_request(&requests) {
                        if completions.send((sequence, request.prepare())).is_err() {
                            break;
                        }
                    }
                });
            match worker {
                Ok(thread) => threads.push(thread),
                Err(error) => bevy::log::warn!(
                    "[viewport-residency] cache worker {slot} unavailable: {error}"
                ),
            }
        }
        let available = !threads.is_empty();
        Self {
            requests: available.then_some(request_tx),
            completions: Some(Mutex::new(completion_rx)),
            threads,
            window: Mutex::default(),
            available: AtomicBool::new(available),
        }
    }

    pub(crate) fn is_available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }

    pub(crate) fn dispatch(
        &self,
        project_root: PathBuf,
        job: LoadJob<ScenePayloadKey>,
    ) -> Result<(), LoadJob<ScenePayloadKey>> {
        self.dispatch_with_payloads(project_root, job, PayloadLoadMask::GEOMETRY, Vec::new())
    }

    pub(crate) fn dispatch_with_payloads(
        &self,
        project_root: PathBuf,
        job: LoadJob<ScenePayloadKey>,
        mask: PayloadLoadMask,
        descriptors: Vec<ScenePayloadDescriptor>,
    ) -> Result<(), LoadJob<ScenePayloadKey>> {
        match self.admit(WorkerRequest::Cached(LoadRequest {
            project_root,
            job,
            mask,
            descriptors,
        })) {
            Ok(()) => Ok(()),
            Err(WorkerRequest::Cached(request)) => Err(request.job),
            Err(WorkerRequest::Source(_)) => unreachable!("rejection preserves the request type"),
        }
    }

    pub(crate) fn dispatch_source(&self, source: SourceMeshInput) -> bool {
        self.admit(WorkerRequest::Source(source)).is_ok()
    }

    pub(super) fn source_capacity(&self) -> Option<SourceWorkCapacity> {
        let window = self.window.lock().ok()?;
        let outstanding = (window.next_request - window.next_publish) as usize;
        let jobs = WORKER_QUEUE_CAPACITY - outstanding;
        let bytes = WORKER_INPUT_BUDGET.saturating_sub(window.reserved_bytes);
        (jobs > 0 && bytes > 0).then_some(SourceWorkCapacity {
            jobs,
            bytes,
            allow_oversized: outstanding == 0,
        })
    }

    fn admit(&self, request: WorkerRequest) -> Result<(), WorkerRequest> {
        if !self.is_available() {
            return Err(request);
        }
        let Some(requests) = self.requests.as_ref() else {
            return Err(request);
        };
        let Ok(mut window) = self.window.lock() else {
            return Err(request);
        };
        let outstanding = window.next_request - window.next_publish;
        let bytes = request.bytes();
        if outstanding >= WORKER_QUEUE_CAPACITY as u64
            || (outstanding > 0
                && (window.reserved_bytes > WORKER_INPUT_BUDGET
                    || bytes > WORKER_INPUT_BUDGET - window.reserved_bytes))
        {
            return Err(request);
        }
        match requests.try_send((window.next_request, request)) {
            Ok(()) => {
                window.next_request += 1;
                window.reserved_bytes += bytes;
                Ok(())
            }
            Err(TrySendError::Full((_, request))) => Err(request),
            Err(TrySendError::Disconnected((_, request))) => {
                self.available.store(false, Ordering::Release);
                Err(request)
            }
        }
    }

    pub(crate) fn drain_completions(&self) -> Vec<WorkerCompletion> {
        let Some(completions) = self.completions.as_ref() else {
            return Vec::new();
        };
        let Ok(receiver) = completions.lock() else {
            return Vec::new();
        };
        let Ok(mut window) = self.window.lock() else {
            return Vec::new();
        };
        let mut completions = Vec::with_capacity(WORKER_QUEUE_CAPACITY);
        while window.ready.len() < WORKER_QUEUE_CAPACITY {
            match receiver.try_recv() {
                Ok((sequence, completion)) => {
                    window.ready.insert(sequence, completion);
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        loop {
            let sequence = window.next_publish;
            let Some(completion) = window.ready.remove(&sequence) else {
                break;
            };
            window.next_publish += 1;
            window.reserved_bytes -= match &completion {
                WorkerCompletion::Cached(completion) => completion.job.cpu_bytes,
                WorkerCompletion::Source { bytes, .. } => *bytes,
            };
            completions.push(completion);
        }
        completions
    }
}

fn receive_request(
    receiver: &Mutex<Receiver<SequencedRequest>>,
) -> Result<SequencedRequest, mpsc::RecvError> {
    receiver.lock().map_err(|_| mpsc::RecvError)?.recv()
}

fn prepare_cached_request(request: LoadRequest) -> LoadCompletion {
    // A bad payload must still occupy its sequence so later work can publish in order.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mesh = if request.mask.geometry {
            load_cached_mesh(&request.project_root, &request.job)?
        } else {
            None
        };
        let payloads = if mesh.is_some() || !request.mask.geometry {
            Some(load_cached_scene_payloads(
                &request.project_root,
                request.mask,
                &request.descriptors,
            )?)
        } else {
            None
        };
        Ok((mesh, payloads))
    }));
    let (result, payloads) = match outcome {
        Ok(Ok((mesh, payloads))) => (Ok(mesh), payloads),
        Ok(Err(error)) => (Err(error), None),
        Err(_) => (Err("cached payload preparation panicked".to_owned()), None),
    };
    LoadCompletion {
        job: request.job,
        mask: request.mask,
        result,
        payloads,
    }
}

/// Queue a decoded completion without acknowledging its residency mask.
/// Return the job on backpressure so the caller can release/requeue it.
pub(crate) fn queue_loaded_scene_payloads(
    queue: &mut LoadedScenePayloadQueue,
    job: LoadJob<ScenePayloadKey>,
    mask: PayloadLoadMask,
    mesh: Option<Mesh>,
    payloads: LoadedScenePayloads,
) -> Result<(), LoadJob<ScenePayloadKey>> {
    if queue.completions.len() >= WORKER_QUEUE_CAPACITY {
        return Err(job);
    }
    queue.completions.push_back(LoadedScenePayloadCompletion {
        job,
        mask,
        mesh,
        payloads,
    });
    Ok(())
}

pub(super) fn reject_scene_payload(
    authority: &mut ResidencyAuthority,
    health: Option<&mut VisualStreamingHealth>,
    job: &LoadJob<ScenePayloadKey>,
    error: &str,
) {
    if authority.suppress_failed_load(job) {
        let message = format!(
            "Scene cache payload {:?} generation {} rejected: {error}",
            job.key, job.generation
        );
        bevy::log::warn!("{message}");
        // Restore camera-unloaded visual owners through the existing native
        // Stage recovery path; never rewrite an immutable cache object to
        // compensate for a device limitation.
        authority
            .visual_composition
            .request_cache_failure_recovery();
        if let Some(health) = health {
            health.incomplete = true;
            health.last_error = Some(message);
        }
    }
}

fn acknowledge_scene_payload(
    authority: &mut ResidencyAuthority,
    job: LoadJob<ScenePayloadKey>,
    mesh: Option<Mesh>,
    mask: PayloadLoadMask,
) -> bool {
    let accepted = match mesh {
        Some(mesh) => authority.complete_cached_cpu_with_mask(job.clone(), mesh, mask),
        None => authority.complete_cached_payloads(&job, mask),
    };
    if accepted {
        authority.clear_repair_phase(&job);
    } else {
        // A stale completion cannot consume a reservation. A current job with
        // an invalid mask still needs its reservation released.
        let _ = authority.suppress_failed_load(&job);
    }
    accepted
}

pub(crate) fn install_loaded_scene_payloads(
    mut queue: bevy::prelude::ResMut<LoadedScenePayloadQueue>,
    presentation: Option<bevy::prelude::Res<crate::viewport::stage::SceneCachePresentation>>,
    render_device: Option<bevy::prelude::Res<bevy::render::renderer::RenderDevice>>,
    mut images: Option<bevy::prelude::ResMut<bevy::asset::Assets<bevy::image::Image>>>,
    mut materials: Option<bevy::prelude::ResMut<bevy::asset::Assets<bevy::pbr::StandardMaterial>>>,
    mut seed: Option<bevy::prelude::ResMut<usd_bevy::ProjectionSeed>>,
    mut texture_cache: Option<bevy::prelude::ResMut<UsdTextureCache>>,
    mut authority: bevy::prelude::ResMut<ResidencyAuthority>,
    mut health: Option<bevy::prelude::ResMut<VisualStreamingHealth>>,
    mut animation_payloads: Option<bevy::prelude::ResMut<SceneAnimationPayloads>>,
    mut commands: bevy::prelude::Commands,
) {
    let Some(presentation) = presentation else {
        while let Some(completion) = queue.completions.pop_front() {
            let _ = authority.defer_load(completion.job);
        }
        return;
    };
    let mut new_animation_payloads: Option<SceneAnimationPayloads> = None;
    while let Some(completion) = queue.completions.pop_front() {
        if completion.job.key.scene_id != presentation.scene_id
            || completion.job.generation != presentation.generation
            || !authority.load_is_current(&completion.job)
        {
            let _ = authority.defer_load(completion.job);
            continue;
        }
        if completion.payloads.materials.is_empty() && completion.payloads.textures.is_empty() {
            if !acknowledge_scene_payload(
                &mut authority,
                completion.job,
                completion.mesh,
                completion.mask,
            ) {
                continue;
            }
        } else {
            let (Some(images), Some(materials), Some(seed), Some(texture_cache)) = (
                images.as_deref_mut(),
                materials.as_deref_mut(),
                seed.as_deref_mut(),
                texture_cache.as_deref_mut(),
            ) else {
                reject_scene_payload(
                    &mut authority,
                    health.as_deref_mut(),
                    &completion.job,
                    "required image/material/seed/metadata resource unavailable",
                );
                continue;
            };
            // Validate every decoded texture against the active device before
            // allocating assets or acknowledging any part of this completion.
            let prepared = completion
                .payloads
                .textures
                .into_iter()
                .map(|(id, texture)| {
                    let axes = texture.authored_black_axes();
                    let image = texture
                        .into_image(render_device.as_deref())
                        .map_err(|error| format!("texture {id}: {error:#}"))?;
                    Ok((id, image, axes))
                })
                .collect::<Result<Vec<_>, String>>();
            let prepared = match prepared {
                Ok(prepared) => prepared,
                Err(error) => {
                    reject_scene_payload(
                        &mut authority,
                        health.as_deref_mut(),
                        &completion.job,
                        &error,
                    );
                    continue;
                }
            };
            let mut texture_handles = std::collections::HashMap::with_capacity(prepared.len());
            let mut authored_axes = Vec::with_capacity(prepared.len());
            for (id, image, axes) in prepared {
                let handle = images.add(image);
                authored_axes.push((handle.clone(), axes));
                texture_handles.insert(id, handle);
            }
            // Construct every material before publishing seeds. On failure, drop
            // the entire temporary batch and leave the canonical source route intact.
            let prepared_materials = completion
                .payloads
                .materials
                .iter()
                .map(|(prim_path, material)| {
                    crate::cache::hydration::standard_material(material, &texture_handles)
                        .map(|value| (prim_path.clone(), value))
                        .map_err(|error| format!("material {prim_path}: {error:#}"))
                })
                .collect::<Result<Vec<_>, String>>();
            let prepared_materials = match prepared_materials {
                Ok(prepared) => prepared,
                Err(error) => {
                    for (handle, _) in authored_axes {
                        let _ = images.remove(handle.id());
                    }
                    reject_scene_payload(
                        &mut authority,
                        health.as_deref_mut(),
                        &completion.job,
                        &error,
                    );
                    continue;
                }
            };
            if !acknowledge_scene_payload(
                &mut authority,
                completion.job,
                completion.mesh,
                completion.mask,
            ) {
                for (handle, _) in authored_axes {
                    let _ = images.remove(handle.id());
                }
                continue;
            }
            texture_cache.prune_authored_black_borders(images);
            for (handle, axes) in authored_axes {
                texture_cache.remember_authored_black_border(&handle, axes);
            }
            for (prim_path, material) in prepared_materials {
                seed.insert_authoritative_material(prim_path, materials.add(material));
            }
        }
        if !completion.payloads.animations.is_empty() {
            if let Some(payloads) = animation_payloads.as_deref_mut() {
                if payloads.scene_id != Some(presentation.scene_id)
                    || payloads.generation != Some(presentation.generation)
                {
                    payloads.scene_id = Some(presentation.scene_id);
                    payloads.generation = Some(presentation.generation);
                    payloads.by_address.clear();
                }
                payloads.by_address.extend(completion.payloads.animations);
            } else {
                let payloads =
                    new_animation_payloads.get_or_insert_with(|| SceneAnimationPayloads {
                        scene_id: Some(presentation.scene_id),
                        generation: Some(presentation.generation),
                        by_address: std::collections::HashMap::new(),
                    });
                payloads.by_address.extend(completion.payloads.animations);
            }
        }
    }
    if let Some(payloads) = new_animation_payloads {
        commands.insert_resource(payloads);
    }
}

impl Drop for CachedResidencyWorker {
    fn drop(&mut self) {
        self.available.store(false, Ordering::Release);
        self.requests.take();
        self.completions.take();
        for thread in self.threads.drain(..) {
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
#[path = "../../../../../tests/unit/viewport/residency/worker_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../../tests/unit/viewport/residency/worker_test_support.rs"]
mod test_support;
