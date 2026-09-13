use crate::project::blob_store::BlobStore;
use crate::project::cache::SceneCacheStore;
use crate::project::cache_contract::SceneCacheBlobRef;
use crate::project::cache_scene_payload::{SCENE_ANIMATION_VERSION, SceneAnimationBlob};
use crate::project::runtime_payload::{RuntimeMaterialBlob, RuntimeTextureBlob};

use super::{LoadedScenePayloads, PayloadLoadMask, ScenePayloadDescriptor};

pub(super) fn load_cached_scene_payloads(
    project_root: &std::path::Path,
    mask: PayloadLoadMask,
    descriptors: &[ScenePayloadDescriptor],
) -> Result<LoadedScenePayloads, String> {
    let store = SceneCacheStore::new(project_root);
    let mut materials = Vec::new();
    let mut texture_ids = std::collections::BTreeSet::new();
    let mut animations = Vec::new();
    for descriptor in descriptors {
        if mask.material {
            if let Some(reference) = descriptor.material.as_ref() {
                let bytes = read_scene_blob(&store, descriptor.address.scene_id, reference)?;
                let material: RuntimeMaterialBlob = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("decode Scene material payload: {error}"))?;
                material
                    .validate()
                    .map_err(|error| format!("validate Scene material payload: {error:#}"))?;
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
                materials.push((descriptor.prim_path.clone(), material));
            }
        }
        if mask.animation {
            if let Some(reference) = descriptor.animation.as_ref() {
                let bytes = read_scene_blob(&store, descriptor.address.scene_id, reference)?;
                let animation: SceneAnimationBlob = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("decode Scene animation payload: {error}"))?;
                if animation.version != SCENE_ANIMATION_VERSION {
                    return Err(format!(
                        "unsupported Scene animation payload version {}",
                        animation.version
                    ));
                }
                animation
                    .validate()
                    .map_err(|error| format!("validate Scene animation payload: {error:#}"))?;
                animations.push((descriptor.address.clone(), animation));
            }
        }
    }
    let mut textures = Vec::new();
    for texture_id in texture_ids {
        let reference = SceneCacheBlobRef {
            blob_id: usd_model::BlobId(texture_id.clone()),
            byte_size: 0,
        };
        let scene_id = descriptors
            .first()
            .map_or_else(usd_project::SceneId::new_v4, |descriptor| {
                descriptor.address.scene_id
            });
        let bytes = store
            .object_store(scene_id)
            .map_err(|error| error.to_string())?
            .get(&reference.blob_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Scene material references missing texture {texture_id}"))?;
        let texture: RuntimeTextureBlob = serde_json::from_slice(&bytes)
            .map_err(|error| format!("decode Scene texture payload: {error}"))?;
        texture
            .validate()
            .map_err(|error| format!("validate Scene texture payload: {error:#}"))?;
        textures.push((texture_id, texture));
    }
    Ok(LoadedScenePayloads {
        materials,
        textures,
        animations,
    })
}

fn read_scene_blob(
    store: &SceneCacheStore,
    scene_id: usd_project::SceneId,
    reference: &SceneCacheBlobRef,
) -> Result<Vec<u8>, String> {
    let object_store = store
        .object_store(scene_id)
        .map_err(|error| error.to_string())?;
    let bytes = object_store
        .get(&reference.blob_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Scene cache blob {} is missing", reference.blob_id.0))?;
    if bytes.len() as u64 != reference.byte_size {
        return Err(format!(
            "Scene cache blob {} has unexpected byte size",
            reference.blob_id.0
        ));
    }
    Ok(bytes)
}
