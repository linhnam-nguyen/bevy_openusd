use bevy::asset::Assets;
use bevy::image::Image;
use bevy::pbr::StandardMaterial;
use bevy::prelude::AlphaMode;
use usd_model::BlobId;

use super::{
    PreparedRuntimeBlob, RuntimeAlphaMode, RuntimeMaterialTextures, RuntimeTextureColorSpace,
    RuntimeTexturePayload,
};

pub(super) fn collect_material_textures(
    material: &StandardMaterial,
    images: &Assets<Image>,
    usd_textures: Option<&usd_bevy::route::material::UsdTextureCache>,
) -> Option<(RuntimeMaterialTextures, Vec<PreparedRuntimeBlob>)> {
    let add = |handle: Option<&bevy::asset::Handle<Image>>,
               color_space: RuntimeTextureColorSpace|
     -> Option<(String, PreparedRuntimeBlob)> {
        let handle = handle?.clone();
        let image = images.get(&handle)?;
        let expected_srgb = color_space == RuntimeTextureColorSpace::Srgb;
        if image.texture_descriptor.format.is_srgb() != expected_srgb {
            return None;
        }
        let authored_black =
            usd_textures.map_or([false; 2], |cache| cache.authored_black_border(&handle));
        let bytes = RuntimeTexturePayload::from_image(image, authored_black)?
            .encode()
            .ok()?;
        let blob_id = BlobId(blake3::hash(&bytes).to_hex().to_string());
        Some((blob_id.0.clone(), PreparedRuntimeBlob { blob_id, bytes }))
    };

    let base_color = add(
        material.base_color_texture.as_ref(),
        RuntimeTextureColorSpace::Srgb,
    );
    let normal = add(
        material.normal_map_texture.as_ref(),
        RuntimeTextureColorSpace::Linear,
    );
    let metallic_roughness = add(
        material.metallic_roughness_texture.as_ref(),
        RuntimeTextureColorSpace::Linear,
    );
    let emissive = add(
        material.emissive_texture.as_ref(),
        RuntimeTextureColorSpace::Srgb,
    );
    let occlusion = add(
        material.occlusion_texture.as_ref(),
        RuntimeTextureColorSpace::Linear,
    );
    let textures = RuntimeMaterialTextures {
        base_color: base_color.as_ref().map(|(id, _)| id.clone()),
        normal: normal.as_ref().map(|(id, _)| id.clone()),
        metallic_roughness: metallic_roughness.as_ref().map(|(id, _)| id.clone()),
        emissive: emissive.as_ref().map(|(id, _)| id.clone()),
        occlusion: occlusion.as_ref().map(|(id, _)| id.clone()),
    };
    if [
        material.base_color_texture.as_ref().is_some() && textures.base_color.is_none(),
        material.normal_map_texture.as_ref().is_some() && textures.normal.is_none(),
        material.metallic_roughness_texture.as_ref().is_some()
            && textures.metallic_roughness.is_none(),
        material.emissive_texture.as_ref().is_some() && textures.emissive.is_none(),
        material.occlusion_texture.as_ref().is_some() && textures.occlusion.is_none(),
    ]
    .into_iter()
    .any(|unsupported| unsupported)
    {
        return None;
    }
    let payloads = [base_color, normal, metallic_roughness, emissive, occlusion]
        .into_iter()
        .flatten()
        .map(|(_, payload)| payload)
        .collect();
    Some((textures, payloads))
}

pub(super) fn runtime_alpha_mode(mode: AlphaMode) -> RuntimeAlphaMode {
    match mode {
        AlphaMode::Opaque => RuntimeAlphaMode::Opaque,
        AlphaMode::Mask(cutoff) => RuntimeAlphaMode::Mask { cutoff },
        AlphaMode::Blend => RuntimeAlphaMode::Blend,
        AlphaMode::Premultiplied => RuntimeAlphaMode::Premultiplied,
        AlphaMode::AlphaToCoverage => RuntimeAlphaMode::AlphaToCoverage,
        AlphaMode::Add => RuntimeAlphaMode::Add,
        AlphaMode::Multiply => RuntimeAlphaMode::Multiply,
    }
}
