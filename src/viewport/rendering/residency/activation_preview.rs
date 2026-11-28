//! Disposable cached geometry for warm scene activation while the previous stage is leased.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bevy::asset::{Assets, Handle};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Camera, Camera3d, RenderTarget};
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::{Component, Entity, Image, Resource, Transform, Vec3, World};
use bevy::render::renderer::RenderDevice;
use usd_bevy::{LiveStage, ProgressiveProjectionState, ProjectionReadiness, ProjectionSeed};

use crate::cache::SceneCacheStore;
use crate::cache::blobs;
use crate::cache::contract::{SceneCacheActivation, SceneCacheEntryKind, SceneCacheState};
use crate::cache::retention::CachePinGuard;
use crate::document::runtime::payload::{RuntimeMaterialBlob, RuntimeTexturePayload};
use crate::viewport::rendering::offscreen::OffscreenTarget;
use crate::viewport::stage::SceneCachePresentation;

use super::projection;
use super::spatial::{self, SceneSpatialPayload};
use super::worker;
use super::{PayloadLoadMask, ResidencyReason, ScenePayloadCatalog};

/// The warm preview is an optional fast path for modest, static cached geometry.
const PREVIEW_GEOMETRY_BUDGET: u64 = 16 * 1024 * 1024;
const PREVIEW_RENDER_WAIT: Duration = Duration::from_millis(300);
const PREVIEW_LAYER: usize = 31;

#[derive(Component)]
pub(crate) struct WarmPreviewMesh;

struct PreparedOccurrence {
    payload: SceneSpatialPayload,
    prim_path: String,
    mesh: Mesh,
    local_extent: ([f32; 3], [f32; 3]),
}

pub(crate) struct PreparedCachePreview {
    occurrences: Vec<PreparedOccurrence>,
    materials: Vec<(String, RuntimeMaterialBlob)>,
    textures: Vec<(String, RuntimeTexturePayload)>,
    pin: CachePinGuard,
}

impl PreparedCachePreview {
    pub(crate) fn prepare(
        root: &Path,
        cache: &SceneCacheActivation,
        pin: CachePinGuard,
    ) -> Result<Option<Self>> {
        if cache.descriptor.state != SceneCacheState::Ready
            || cache.descriptor.config_hash
                != crate::cache::hydration::default_project_cache_config_hash()
        {
            return Ok(None);
        }
        let presentation = SceneCachePresentation::from_activation(cache);
        let catalog =
            ScenePayloadCatalog::from_presentation(&presentation).map_err(anyhow::Error::msg)?;
        let entries: HashMap<_, _> = presentation
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (&entry.address, index))
            .collect();
        let payloads = spatial::scene_payloads(&presentation.entries);
        let mut eligible = Vec::new();
        let mut bytes = 0u64;
        for payload in payloads {
            let Some(&index) = entries.get(&payload.address) else {
                continue;
            };
            let entry = &presentation.entries[index];
            let SceneCacheEntryKind::OwnedPrim { prim_path } = &entry.kind else {
                continue;
            };
            if !visible_in_cache(&presentation, index) {
                continue;
            }
            if entry.animation.is_some() {
                return Ok(None);
            }
            bytes = bytes.saturating_add(payload.cpu_bytes);
            if bytes > PREVIEW_GEOMETRY_BUDGET {
                return Ok(None);
            }
            let bounds = entry.bounds.context("missing cache bounds")?;
            let local_extent = (
                bounds.min.map(|value| value as f32),
                bounds.max.map(|value| value as f32),
            );
            eligible.push((
                payload,
                prim_path.clone(),
                entry.geometry.as_ref().context("missing cache geometry")?,
                local_extent,
            ));
        }
        if eligible.is_empty() {
            return Ok(None);
        }
        let store = SceneCacheStore::new(root).object_store(cache.descriptor.scene_id)?;
        let mut occurrences = Vec::with_capacity(eligible.len());
        let mut descriptors = Vec::with_capacity(eligible.len());
        for (payload, prim_path, geometry, local_extent) in eligible {
            let mesh = blobs::get_mesh(&store, &geometry.blob_id)?
                .context("warm preview mesh is absent")?;
            if let Some(descriptor) = catalog
                .descriptors_for(payload.payload_key)
                .iter()
                .find(|descriptor| descriptor.address == payload.address)
            {
                descriptors.push(descriptor.clone());
            }
            occurrences.push(PreparedOccurrence {
                payload,
                prim_path,
                mesh,
                local_extent,
            });
        }
        let loaded = worker::load_cached_scene_payloads(
            root,
            PayloadLoadMask::for_reason(ResidencyReason::CameraNear),
            &descriptors,
        )
        .map_err(anyhow::Error::msg)?;
        Ok(Some(Self {
            occurrences,
            materials: loaded.materials,
            textures: loaded.textures,
            pin,
        }))
    }
}

fn visible_in_cache(presentation: &SceneCachePresentation, index: usize) -> bool {
    let mut next = Some(index);
    while let Some(index) = next {
        let Some(entry) = presentation.entries.get(index) else {
            return false;
        };
        if !entry.hierarchy.metadata_populated || !entry.hierarchy.authored_visible {
            return false;
        }
        next = entry.parent.map(|parent| parent as usize);
    }
    true
}

#[derive(Resource)]
pub(crate) struct WarmScenePreview {
    pub(crate) activation: u64,
    pub(crate) camera: Entity,
    previous_session: u64,
    expires: Instant,
    cameras: Vec<Entity>,
    entities: Vec<Entity>,
    meshes: Vec<(String, Handle<Mesh>, ([f32; 3], [f32; 3]))>,
    _pin: CachePinGuard,
}

impl WarmScenePreview {
    pub(crate) fn install(
        world: &mut World,
        activation: u64,
        prepared: PreparedCachePreview,
    ) -> bool {
        let Some(stage) = world.get_non_send::<LiveStage>() else {
            return false;
        };
        let stage_session = stage.session_id();
        if !world
            .get_resource::<ProgressiveProjectionState>()
            .is_some_and(|projection| {
                projection.session_id() == Some(stage_session)
                    && projection.readiness() == ProjectionReadiness::Ready
            })
            || !world.contains_resource::<OffscreenTarget>()
            || !world.contains_resource::<Assets<Image>>()
            || !world.contains_resource::<Assets<Mesh>>()
            || !world.contains_resource::<Assets<StandardMaterial>>()
        {
            return false;
        }
        let output = world.resource::<OffscreenTarget>().image_handle.clone();
        let cameras: Vec<_> = world
            .query::<(Entity, &Camera, &Camera3d, &RenderTarget)>()
            .iter(world)
            .filter_map(|(entity, camera, _, target)| {
                (camera.is_active && target.as_image() == Some(&output)).then_some(entity)
            })
            .collect();
        if cameras.is_empty() {
            return false;
        }
        let Some(camera_transform) = preview_camera(&prepared.occurrences) else {
            return false;
        };
        let (images, material_values) =
            match prepare_materials(world, prepared.textures, prepared.materials) {
                Ok(values) => values,
                Err(error) => {
                    bevy::log::debug!(
                        "[project-cache] warm material preview unavailable: {error:#}"
                    );
                    return false;
                }
            };
        let Some(stage) = world.get_non_send::<LiveStage>() else {
            return false;
        };
        if !stage.try_freeze_authoring() {
            return false;
        }

        for &entity in &cameras {
            if let Some(mut camera) = world.get_mut::<Camera>(entity) {
                camera.is_active = false;
            }
        }
        let layer = RenderLayers::layer(PREVIEW_LAYER);
        let camera = world
            .spawn((
                Camera3d::default(),
                RenderTarget::Image(output.into()),
                camera_transform,
                layer.clone(),
            ))
            .id();
        let mut entities = vec![camera];
        let fallback = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                unlit: true,
                ..Default::default()
            });
        let mut material_handles: HashMap<_, _> = material_values
            .into_iter()
            .map(|(path, material)| {
                (
                    path,
                    world
                        .resource_mut::<Assets<StandardMaterial>>()
                        .add(material),
                )
            })
            .collect();
        let mut meshes = Vec::new();
        for occurrence in prepared.occurrences {
            let mesh = world.resource_mut::<Assets<Mesh>>().add(occurrence.mesh);
            let material = material_handles
                .remove(&occurrence.prim_path)
                .unwrap_or_else(|| fallback.clone());
            entities.push(
                world
                    .spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material),
                        WarmPreviewMesh,
                        projection::transform_for_cache(&occurrence.payload.transform),
                        layer.clone(),
                    ))
                    .id(),
            );
            meshes.push((occurrence.prim_path, mesh, occurrence.local_extent));
        }
        // The prepared material values and entities own the needed texture handles.
        drop(images);
        world.insert_resource(Self {
            activation,
            camera,
            previous_session: stage_session,
            expires: Instant::now() + PREVIEW_RENDER_WAIT,
            cameras,
            entities,
            meshes,
            _pin: prepared.pin,
        });
        true
    }

    pub(crate) fn may_open(world: &World, activation: u64) -> bool {
        world.get_resource::<Self>().is_none_or(|preview| {
            preview.activation != activation
                || Instant::now() >= preview.expires
                || crate::viewport::rendering::offscreen::projection_readback::completed_preview(
                    world, activation,
                )
        })
    }

    pub(crate) fn take(world: &mut World, activation: u64) -> Option<Self> {
        if !world
            .get_resource::<Self>()
            .is_some_and(|preview| preview.activation == activation)
        {
            return None;
        }
        let preview = world.remove_resource::<Self>()?;
        for &entity in &preview.entities {
            let _ = world.despawn(entity);
        }
        for &entity in &preview.cameras {
            if let Some(mut camera) = world.get_mut::<Camera>(entity) {
                camera.is_active = true;
            }
        }
        if let Some(stage) = world.get_non_send::<LiveStage>()
            && stage.session_id() == preview.previous_session
        {
            stage.unfreeze_authoring();
        }
        Some(preview)
    }

    pub(crate) fn reuse_meshes(self, world: &mut World) {
        if let Some(mut seed) = world.get_resource_mut::<ProjectionSeed>() {
            for (path, mesh, extent) in self.meshes {
                seed.insert_mesh(path, mesh, Some(extent));
            }
        }
    }
}

type PreparedMaterials = (
    HashMap<String, Handle<Image>>,
    Vec<(String, StandardMaterial)>,
);

fn prepare_materials(
    world: &mut World,
    textures: Vec<(String, RuntimeTexturePayload)>,
    materials: Vec<(String, RuntimeMaterialBlob)>,
) -> Result<PreparedMaterials> {
    let decoded = textures
        .into_iter()
        .map(|(id, texture)| {
            Ok((
                id,
                texture.into_image(world.get_resource::<RenderDevice>())?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let handles: HashMap<_, _> = decoded
        .into_iter()
        .map(|(id, texture)| (id, world.resource_mut::<Assets<Image>>().add(texture)))
        .collect();
    let result = materials
        .iter()
        .map(|(path, value)| {
            let mut material = crate::cache::hydration::standard_material(value, &handles)?;
            material.unlit = true;
            Ok((path.clone(), material))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((handles, result))
}

fn preview_camera(occurrences: &[PreparedOccurrence]) -> Option<Transform> {
    let (min, max) = occurrences.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(min, max), occurrence| {
            let bounds = occurrence.payload.bounds;
            (
                min.min(Vec3::from_array(bounds.min.map(|value| value as f32))),
                max.max(Vec3::from_array(bounds.max.map(|value| value as f32))),
            )
        },
    );
    let center = min.midpoint(max);
    let distance = (max - min).length().max(0.1) * 1.5;
    if !center.is_finite() || !distance.is_finite() {
        return None;
    }
    Some(
        Transform::from_translation(center + Vec3::new(1.0, 0.65, 1.0).normalize() * distance)
            .looking_at(center, Vec3::Y),
    )
}

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/residency/activation_preview_tests.rs"]
mod tests;
