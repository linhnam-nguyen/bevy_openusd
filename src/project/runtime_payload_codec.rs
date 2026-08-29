use bevy::asset::Assets;
use bevy::image::Image;
use bevy::pbr::StandardMaterial;
use bevy::prelude::AlphaMode;
use bevy::render::render_resource::{TextureDimension, TextureFormat};
use usd_model::BlobId;

use super::{
    PreparedRuntimeBlob, RUNTIME_TEXTURE_VERSION, RuntimeAlphaMode, RuntimeMaterialTextures,
    RuntimeTextureBlob, RuntimeTextureColorSpace,
};

pub(super) fn collect_material_textures(
    material: &StandardMaterial,
    images: &Assets<Image>,
) -> Option<(RuntimeMaterialTextures, Vec<PreparedRuntimeBlob>)> {
    let add = |handle: Option<&bevy::asset::Handle<Image>>,
               color_space: RuntimeTextureColorSpace|
     -> Option<(String, PreparedRuntimeBlob)> {
        let handle = handle?.clone();
        let image = images.get(&handle)?;
        let data = image.data.as_ref()?;
        if image.texture_descriptor.dimension != TextureDimension::D2
            || image.texture_descriptor.format
                != match color_space {
                    RuntimeTextureColorSpace::Srgb => TextureFormat::Rgba8UnormSrgb,
                    RuntimeTextureColorSpace::Linear => TextureFormat::Rgba8Unorm,
                }
            || image.texture_descriptor.mip_level_count != 1
            || image.texture_descriptor.size.depth_or_array_layers != 1
        {
            return None;
        }
        let payload = RuntimeTextureBlob {
            version: RUNTIME_TEXTURE_VERSION,
            color_space,
            width: image.texture_descriptor.size.width,
            height: image.texture_descriptor.size.height,
            rgba8: data.clone(),
        };
        payload.validate().ok()?;
        let bytes = serde_json::to_vec(&payload).ok()?;
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
