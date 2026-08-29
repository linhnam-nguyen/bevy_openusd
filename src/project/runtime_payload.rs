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
use bevy::prelude::World;
use serde::{Deserialize, Serialize};
use usd_bevy::UsdPrimRef;
use usd_model::BlobId;

#[path = "runtime_payload_codec.rs"]
mod codec;
use codec::{collect_material_textures, runtime_alpha_mode};

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
    prepare_runtime_payloads_with_filter(world, snapshot, |_| true)
}

/// Capture only authored material bindings for a canonical Stage warm.
///
/// The normal route attaches one shared fallback material to unbound meshes;
/// that presentation asset is not a source material conversion and must not
/// become a persistent material seed that the material route cannot consume.
pub(crate) fn prepare_runtime_payloads_for_stage(
    world: &mut World,
    stage: &openusd::usd::Stage,
    snapshot: &usd_model::SemanticSnapshot,
) -> PreparedRuntimePayloads {
    prepare_runtime_payloads_with_filter(world, snapshot, |path| {
        let Ok(path) = openusd::sdf::path(path) else {
            return false;
        };
        matches!(
            usd_bevy::read::shade::read_material_binding(stage, &path),
            Ok(Some(_))
        )
    })
}

fn prepare_runtime_payloads_with_filter<F: Fn(&str) -> bool>(
    world: &mut World,
    snapshot: &usd_model::SemanticSnapshot,
    include: F,
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
            .filter(|(prim, _)| {
                snapshot_paths.contains(prim.path.as_str()) && include(prim.path.as_str())
            })
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
