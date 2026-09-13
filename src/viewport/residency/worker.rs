//! Bounded cache read/decode work for viewport residency.

use std::collections::VecDeque;
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
use crate::project::cache_contract::SceneCacheAddress;
use crate::project::cache_scene_payload::SceneAnimationBlob;
use crate::project::runtime_payload::{RuntimeMaterialBlob, RuntimeTextureBlob};
use crate::project::cache_scene_hydration::SceneAnimationPayloads;

use super::{PayloadLoadMask, ScenePayloadDescriptor, ScenePayloadKey};
use super::loader::LoadJob;

#[path = "worker_payloads.rs"]
mod payloads;
use payloads::load_cached_scene_payloads;

pub(crate) const WORKER_QUEUE_CAPACITY: usize = 8;

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
    pub(crate) textures: Vec<(String, RuntimeTextureBlob)>,
    pub(crate) animations: Vec<(SceneCacheAddress, SceneAnimationBlob)>,
}

#[derive(Resource, Default)]
pub(crate) struct LoadedScenePayloadQueue {
    completions: VecDeque<LoadedScenePayloadCompletion>,
}

struct LoadedScenePayloadCompletion {
    job: LoadJob<ScenePayloadKey>,
    payloads: LoadedScenePayloads,
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
                    let result = if request.mask.geometry {
                        load_cached_mesh(&request.project_root, &request.job)
                    } else {
                        Ok(None)
                    };
                    let (result, payloads) = match result {
                        Ok(Some(mesh)) => match load_cached_scene_payloads(
                            &request.project_root,
                            request.mask,
                            &request.descriptors,
                        ) {
                            Ok(payloads) => (Ok(Some(mesh)), Some(payloads)),
                            Err(error) => (Err(error), None),
                        },
                        Ok(None) if !request.mask.geometry => match load_cached_scene_payloads(
                            &request.project_root,
                            request.mask,
                            &request.descriptors,
                        ) {
                            Ok(payloads) => (Ok(None), Some(payloads)),
                            Err(error) => (Err(error), None),
                        },
                        Ok(None) => (Ok(None), None),
                        Err(error) => (Err(error), None),
                    };
                    if completion_tx
                        .send(LoadCompletion {
                            job: request.job,
                            mask: request.mask,
                            result,
                            payloads,
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

    #[cfg(test)]
    pub(crate) fn with_completion_for_test(completion: LoadCompletion) -> Self {
        let (completion_tx, completion_rx) = mpsc::sync_channel(WORKER_QUEUE_CAPACITY);
        completion_tx
            .send(completion)
            .expect("test completion queue accepts one completion");
        drop(completion_tx);
        Self {
            requests: None,
            completions: Some(Mutex::new(completion_rx)),
            thread: None,
            available: AtomicBool::new(false),
        }
    }

    pub(crate) fn dispatch(
        &self,
        project_root: PathBuf,
        job: LoadJob<ScenePayloadKey>,
    ) -> Result<(), LoadJob<ScenePayloadKey>> {
        self.dispatch_with_payloads(
            project_root,
            job,
            PayloadLoadMask::GEOMETRY,
            Vec::new(),
        )
    }

    pub(crate) fn dispatch_with_payloads(
        &self,
        project_root: PathBuf,
        job: LoadJob<ScenePayloadKey>,
        mask: PayloadLoadMask,
        descriptors: Vec<ScenePayloadDescriptor>,
    ) -> Result<(), LoadJob<ScenePayloadKey>> {
        if !self.is_available() {
            return Err(job);
        }
        let Some(requests) = self.requests.as_ref() else {
            return Err(job);
        };
        let request = LoadRequest {
            project_root,
            job,
            mask,
            descriptors,
        };
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

pub(crate) fn queue_loaded_scene_payloads(
    queue: &mut LoadedScenePayloadQueue,
    job: LoadJob<ScenePayloadKey>,
    payloads: LoadedScenePayloads,
) {
    if queue.completions.len() < WORKER_QUEUE_CAPACITY {
        queue
            .completions
            .push_back(LoadedScenePayloadCompletion { job, payloads });
    }
}

pub(crate) fn install_loaded_scene_payloads(
    mut queue: bevy::prelude::ResMut<LoadedScenePayloadQueue>,
    presentation: Option<bevy::prelude::Res<crate::viewport::session::SceneCachePresentation>>,
    mut images: Option<bevy::prelude::ResMut<bevy::asset::Assets<bevy::image::Image>>>,
    mut materials: Option<
        bevy::prelude::ResMut<bevy::asset::Assets<bevy::pbr::StandardMaterial>>,
    >,
    mut seed: Option<bevy::prelude::ResMut<usd_bevy::ProjectionSeed>>,
    mut animation_payloads: Option<bevy::prelude::ResMut<SceneAnimationPayloads>>,
    mut commands: bevy::prelude::Commands,
) {
    let Some(presentation) = presentation else {
        queue.completions.clear();
        return;
    };
    let Some(images) = images.as_deref_mut() else {
        return;
    };
    let Some(materials) = materials.as_deref_mut() else {
        return;
    };
    let Some(seed) = seed.as_deref_mut() else {
        return;
    };

    while let Some(completion) = queue.completions.pop_front() {
        if completion.job.key.scene_id != presentation.scene_id
            || completion.job.generation != presentation.generation
        {
            continue;
        }
        let texture_handles = completion
            .payloads
            .textures
            .iter()
            .map(|(id, texture)| {
                let format = match texture.color_space {
                    crate::project::runtime_payload::RuntimeTextureColorSpace::Srgb => {
                        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb
                    }
                    crate::project::runtime_payload::RuntimeTextureColorSpace::Linear => {
                        bevy::render::render_resource::TextureFormat::Rgba8Unorm
                    }
                };
                let image = bevy::image::Image::new(
                    bevy::render::render_resource::Extent3d {
                        width: texture.width,
                        height: texture.height,
                        depth_or_array_layers: 1,
                    },
                    bevy::render::render_resource::TextureDimension::D2,
                    texture.rgba8.clone(),
                    format,
                    bevy::asset::RenderAssetUsages::default(),
                );
                (id.clone(), images.add(image))
            })
            .collect::<std::collections::HashMap<_, _>>();
        let mut material_handles = Vec::with_capacity(completion.payloads.materials.len());
        for (prim_path, material) in &completion.payloads.materials {
            let Ok(material) = crate::project::cache_hydration::standard_material(
                material,
                &texture_handles,
            ) else {
                continue;
            };
            material_handles.push((prim_path.clone(), materials.add(material)));
        }
        for (prim_path, handle) in material_handles {
            seed.insert_authoritative_material(prim_path, handle);
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
                let mut payloads = SceneAnimationPayloads {
                    scene_id: Some(presentation.scene_id),
                    generation: Some(presentation.generation),
                    by_address: std::collections::HashMap::new(),
                };
                payloads.by_address.extend(completion.payloads.animations);
                commands.insert_resource(payloads);
            }
        }
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
#[path = "worker_tests.rs"]
mod tests;
