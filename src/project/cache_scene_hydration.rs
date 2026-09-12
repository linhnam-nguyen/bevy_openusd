//! Scene-V3 material/texture hydration and animation-payload residency.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result, ensure};
use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::mesh::Mesh;
use bevy::pbr::StandardMaterial;
use bevy::prelude::World;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use usd_model::BlobId;

use super::blob_store::{BlobStore, FilesystemBlobStore};
use super::cache_contract::{SceneCacheActivation, SceneCacheBlobRef, SceneCacheEntryKind};
use super::cache_scene_payload::{SceneAnimationBlob, SCENE_ANIMATION_VERSION};
use super::cache_hydration::standard_material;

#[derive(bevy::ecs::resource::Resource, Clone, Debug, Default)]
pub(crate) struct SceneAnimationPayloads {
    pub(crate) scene_id: Option<usd_project::SceneId>,
    pub(crate) generation: Option<u64>,
    pub(crate) by_address:
        HashMap<super::cache_contract::SceneCacheAddress, SceneAnimationBlob>,
}

/// Hydrate the actual Scene-owned material/texture blobs into the normal
/// renderer assets and load animation blobs for the residency consumer. A
/// missing renderer resource is a normal source-fallback boundary.
pub(crate) fn hydrate_scene_cache_payloads(
    world: &mut World,
    project_root: &Path,
    activation: &SceneCacheActivation,
) -> Result<bool> {
    let store = FilesystemBlobStore::new(
        super::storage::ProjectStorageLayout::new(project_root)
            .scene_cache_objects_dir(activation.descriptor.scene_id),
    )?;
    let mut material_payloads = HashMap::new();
    let mut texture_ids = HashSet::new();
    for entry in &activation.index.entries {
        let SceneCacheEntryKind::OwnedPrim { .. } = &entry.kind else {
            continue;
        };
        let Some(reference) = entry.material.as_ref() else {
            continue;
        };
        let bytes = read_scene_blob(&store, reference)?;
        let material: super::runtime_payload::RuntimeMaterialBlob =
            serde_json::from_slice(&bytes).context("decode Scene material payload")?;
        material.validate()?;
        for texture_id in [
            material.textures.base_color.as_deref(),
            material.textures.normal.as_deref(),
            material.textures.metallic_roughness.as_deref(),
            material.textures.emissive.as_deref(),
            material.textures.occlusion.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            texture_ids.insert(texture_id.to_owned());
        }
        material_payloads.insert(reference.blob_id.0.clone(), material);
    }

    let mut texture_assets = HashMap::new();
    for texture_id in texture_ids {
        let reference = SceneCacheBlobRef {
            blob_id: BlobId(texture_id.clone()),
            byte_size: 0,
        };
        let bytes = find_scene_blob(&store, &reference)?
            .with_context(|| format!("Scene material references missing texture {texture_id}"))?;
        let texture: super::runtime_payload::RuntimeTextureBlob =
            serde_json::from_slice(&bytes).context("decode Scene texture payload")?;
        texture.validate()?;
        let format = match texture.color_space {
            super::runtime_payload::RuntimeTextureColorSpace::Srgb => {
                TextureFormat::Rgba8UnormSrgb
            }
            super::runtime_payload::RuntimeTextureColorSpace::Linear => TextureFormat::Rgba8Unorm,
        };
        texture_assets.insert(
            texture_id,
            Image::new(
                Extent3d {
                    width: texture.width,
                    height: texture.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                texture.rgba8,
                format,
                RenderAssetUsages::default(),
            ),
        );
    }

    let mut animation_payloads = SceneAnimationPayloads {
        scene_id: Some(activation.descriptor.scene_id),
        generation: Some(activation.descriptor.generation),
        by_address: HashMap::new(),
    };
    for entry in &activation.index.entries {
        let Some(reference) = entry.animation.as_ref() else {
            continue;
        };
        let bytes = read_scene_blob(&store, reference)?;
        let animation: SceneAnimationBlob =
            serde_json::from_slice(&bytes).context("decode Scene animation payload")?;
        ensure!(
            animation.version == SCENE_ANIMATION_VERSION,
            "unsupported Scene animation payload version {}",
            animation.version
        );
        animation.validate()?;
        animation_payloads
            .by_address
            .insert(entry.address.clone(), animation);
    }
    world.insert_resource(animation_payloads);

    if material_payloads.is_empty() {
        return Ok(false);
    }
    let (Some(_), Some(_), Some(_), Some(_)) = (
        world.get_resource::<bevy::asset::Assets<Mesh>>(),
        world.get_resource::<bevy::asset::Assets<Image>>(),
        world.get_resource::<bevy::asset::Assets<StandardMaterial>>(),
        world.get_resource::<usd_bevy::ProjectionSeed>(),
    ) else {
        return Ok(false);
    };
    let texture_handles = {
        let mut assets = world.resource_mut::<bevy::asset::Assets<Image>>();
        texture_assets
            .into_iter()
            .map(|(id, image)| (id, assets.add(image)))
            .collect::<HashMap<_, _>>()
    };
    let material_handles = {
        let mut assets = world.resource_mut::<bevy::asset::Assets<StandardMaterial>>();
        material_payloads
            .into_iter()
            .map(|(id, material)| {
                Ok((id, assets.add(standard_material(&material, &texture_handles)?)))
            })
            .collect::<Result<HashMap<_, _>>>()?
    };
    let Some(mut seed) = world.get_resource_mut::<usd_bevy::ProjectionSeed>() else {
        return Ok(false);
    };
    for entry in &activation.index.entries {
        let SceneCacheEntryKind::OwnedPrim { prim_path } = &entry.kind else {
            continue;
        };
        let Some(reference) = entry.material.as_ref() else {
            continue;
        };
        let Some(handle) = material_handles.get(&reference.blob_id.0) else {
            continue;
        };
        seed.insert_authoritative_material(prim_path.clone(), handle.clone());
    }
    Ok(true)
}

fn read_scene_blob(store: &FilesystemBlobStore, reference: &SceneCacheBlobRef) -> Result<Vec<u8>> {
    store
        .get(&reference.blob_id)?
        .with_context(|| format!("Scene cache blob {} is missing", reference.blob_id.0))
        .and_then(|bytes| {
            ensure!(
                bytes.len() as u64 == reference.byte_size,
                "Scene cache blob {} has unexpected byte size",
                reference.blob_id.0
            );
            Ok(bytes)
        })
}

fn find_scene_blob(
    store: &FilesystemBlobStore,
    reference: &SceneCacheBlobRef,
) -> Result<Option<Vec<u8>>> {
    store.get(&reference.blob_id)
}
