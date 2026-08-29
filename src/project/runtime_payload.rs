//! Versioned, renderer-neutral material and texture payloads.
//!
//! These values are produced from the existing USD -> Bevy material route by
//! the application adapter.  They deliberately contain no Bevy handles or
//! filesystem paths.  Texture interpretation is part of the payload bytes so
//! identical source pixels used as color data and linear data get different
//! content addresses.

use std::collections::{BTreeMap, HashSet};

use bevy::asset::Assets;
use bevy::image::Image;
use bevy::pbr::StandardMaterial;
use bevy::prelude::{AlphaMode, World};
use bevy::render::render_resource::{TextureDimension, TextureFormat};
use serde::{Deserialize, Serialize};
use usd_bevy::UsdPrimRef;
use usd_model::BlobId;

pub(crate) const RUNTIME_TEXTURE_VERSION: u16 = 1;
pub(crate) const RUNTIME_MATERIAL_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeTextureColorSpace {
    Srgb,
    Linear,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct RuntimeTextureBlob {
    pub(crate) version: u16,
    pub(crate) color_space: RuntimeTextureColorSpace,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba8: Vec<u8>,
}

impl RuntimeTextureBlob {
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.version == RUNTIME_TEXTURE_VERSION,
            "unsupported runtime texture blob version {}",
            self.version
        );
        anyhow::ensure!(
            self.width > 0 && self.height > 0,
            "runtime texture has no extent"
        );
        let expected = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| anyhow::anyhow!("runtime texture extent overflows byte count"))?;
        anyhow::ensure!(
            self.rgba8.len() == expected,
            "runtime texture has {} bytes, expected {}",
            self.rgba8.len(),
            expected
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeAlphaMode {
    Opaque,
    Mask { cutoff: f32 },
    Blend,
    Premultiplied,
    AlphaToCoverage,
    Add,
    Multiply,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub(crate) struct RuntimeMaterialTextures {
    pub(crate) base_color: Option<String>,
    pub(crate) normal: Option<String>,
    pub(crate) metallic_roughness: Option<String>,
    pub(crate) emissive: Option<String>,
    pub(crate) occlusion: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct RuntimeMaterialBlob {
    pub(crate) version: u16,
    pub(crate) base_color: [f32; 4],
    pub(crate) emissive: [f32; 4],
    pub(crate) perceptual_roughness: f32,
    pub(crate) metallic: f32,
    pub(crate) ior: f32,
    pub(crate) alpha_mode: RuntimeAlphaMode,
    pub(crate) double_sided: bool,
    pub(crate) unlit: bool,
    pub(crate) uv_transform: [[f32; 2]; 3],
    pub(crate) textures: RuntimeMaterialTextures,
}

impl RuntimeMaterialBlob {
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.version == RUNTIME_MATERIAL_VERSION,
            "unsupported runtime material blob version {}",
            self.version
        );
        anyhow::ensure!(
            self.base_color
                .iter()
                .chain(self.emissive.iter())
                .all(|value| value.is_finite())
                && self.perceptual_roughness.is_finite()
                && self.metallic.is_finite()
                && self.ior.is_finite()
                && self
                    .uv_transform
                    .iter()
                    .flatten()
                    .all(|value| value.is_finite()),
            "runtime material contains a non-finite value"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedRuntimeBlob {
    pub(crate) blob_id: BlobId,
    pub(crate) bytes: Vec<u8>,
}

/// Payloads prepared from one complete projected snapshot.
#[derive(Clone, Debug)]
pub(crate) struct PreparedRuntimePayloads {
    pub(crate) material_by_entity: BTreeMap<String, String>,
    pub(crate) materials: Vec<PreparedRuntimeBlob>,
    pub(crate) textures: Vec<PreparedRuntimeBlob>,
    pub(crate) complete: bool,
}

impl Default for PreparedRuntimePayloads {
    fn default() -> Self {
        Self {
            material_by_entity: BTreeMap::new(),
            materials: Vec::new(),
            textures: Vec::new(),
            complete: true,
        }
    }
}

/// Capture material and texture assets after the existing USD material route
/// has decoded them. Unsupported image layouts intentionally omit the
/// material from this cache batch so the source projection remains the safe
/// fallback.
pub(crate) fn prepare_runtime_payloads(
    world: &mut World,
    snapshot: &usd_model::SemanticSnapshot,
) -> PreparedRuntimePayloads {
    let snapshot_paths = snapshot
        .entities
        .values()
        .map(|entity| entity.prim_path.as_str())
        .collect::<HashSet<_>>();
    let bindings = {
        let mut query =
            world.query::<(&UsdPrimRef, &bevy::pbr::MeshMaterial3d<StandardMaterial>)>();
        query
            .iter(world)
            .filter(|(prim, _)| snapshot_paths.contains(prim.path.as_str()))
            .map(|(prim, material)| (prim.path.clone(), material.0.clone()))
            .collect::<Vec<_>>()
    };
    let Some(materials) = world.get_resource::<Assets<StandardMaterial>>() else {
        return PreparedRuntimePayloads {
            complete: bindings.is_empty(),
            ..Default::default()
        };
    };
    let Some(images) = world.get_resource::<Assets<Image>>() else {
        return PreparedRuntimePayloads {
            complete: bindings.is_empty(),
            ..Default::default()
        };
    };

    let mut prepared = PreparedRuntimePayloads {
        complete: true,
        ..Default::default()
    };
    for (entity_path, handle) in bindings {
        let Some(material) = materials.get(&handle) else {
            prepared.complete = false;
            continue;
        };
        let Some((textures, texture_payloads)) = collect_material_textures(material, images) else {
            prepared.complete = false;
            continue;
        };
        let descriptor = RuntimeMaterialBlob {
            version: RUNTIME_MATERIAL_VERSION,
            base_color: {
                let color = material.base_color.to_srgba();
                [color.red, color.green, color.blue, color.alpha]
            },
            emissive: [
                material.emissive.red,
                material.emissive.green,
                material.emissive.blue,
                material.emissive.alpha,
            ],
            perceptual_roughness: material.perceptual_roughness,
            metallic: material.metallic,
            ior: material.ior,
            alpha_mode: runtime_alpha_mode(material.alpha_mode),
            double_sided: material.double_sided,
            unlit: material.unlit,
            uv_transform: [
                [
                    material.uv_transform.matrix2.x_axis.x,
                    material.uv_transform.matrix2.x_axis.y,
                ],
                [
                    material.uv_transform.matrix2.y_axis.x,
                    material.uv_transform.matrix2.y_axis.y,
                ],
                [
                    material.uv_transform.translation.x,
                    material.uv_transform.translation.y,
                ],
            ],
            textures,
        };
        if descriptor.validate().is_err() {
            prepared.complete = false;
            continue;
        }
        let Ok(bytes) = serde_json::to_vec(&descriptor) else {
            continue;
        };
        let blob_id = BlobId(blake3::hash(&bytes).to_hex().to_string());
        if !prepared
            .materials
            .iter()
            .any(|candidate| candidate.blob_id == blob_id)
        {
            prepared.materials.push(PreparedRuntimeBlob {
                blob_id: blob_id.clone(),
                bytes,
            });
        }
        prepared.material_by_entity.insert(entity_path, blob_id.0);
        for texture in texture_payloads {
            if !prepared
                .textures
                .iter()
                .any(|candidate| candidate.blob_id == texture.blob_id)
            {
                prepared.textures.push(texture);
            }
        }
    }

    // A material is usable only when every texture it references was prepared
    // in this batch. This keeps a future Ready descriptor honest and lets the
    // source route handle missing or unsupported images.
    let texture_ids = prepared
        .textures
        .iter()
        .map(|payload| payload.blob_id.0.as_str())
        .collect::<HashSet<_>>();
    let material_ids = prepared
        .materials
        .iter()
        .filter_map(|payload| {
            let descriptor = serde_json::from_slice::<RuntimeMaterialBlob>(&payload.bytes).ok()?;
            let references = [
                descriptor.textures.base_color.as_deref(),
                descriptor.textures.normal.as_deref(),
                descriptor.textures.metallic_roughness.as_deref(),
                descriptor.textures.emissive.as_deref(),
                descriptor.textures.occlusion.as_deref(),
            ];
            references
                .iter()
                .flatten()
                .all(|blob_id| texture_ids.contains(blob_id))
                .then_some(payload.blob_id.0.clone())
        })
        .collect::<HashSet<_>>();
    prepared
        .material_by_entity
        .retain(|_, blob_id| material_ids.contains(blob_id));
    prepared
}

fn collect_material_textures(
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

fn runtime_alpha_mode(mode: AlphaMode) -> RuntimeAlphaMode {
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
