//! Validated, lazy Scene payload metadata for residency demand.

use std::collections::HashMap;

use bevy::prelude::Resource;
use usd_project::SceneId;

use crate::project::cache_contract::{SceneCacheAddress, SceneCacheBlobRef, SceneCacheEntryKind};
use crate::viewport::session::SceneCachePresentation;

use super::{ResidencyReason, ScenePayloadKey};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PayloadLoadMask {
    pub(crate) geometry: bool,
    pub(crate) material: bool,
    pub(crate) animation: bool,
}

impl PayloadLoadMask {
    pub(crate) const GEOMETRY: Self = Self {
        geometry: true,
        material: false,
        animation: false,
    };

    pub(crate) fn for_reason(reason: ResidencyReason) -> Self {
        match reason {
            ResidencyReason::CameraNear
            | ResidencyReason::Selected
            | ResidencyReason::ActiveViewpoint => Self {
                geometry: true,
                material: true,
                animation: false,
            },
            ResidencyReason::AnimationRequired => Self {
                geometry: true,
                material: true,
                animation: true,
            },
        }
    }

    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            geometry: self.geometry || other.geometry,
            material: self.material || other.material,
            animation: self.animation || other.animation,
        }
    }

    pub(crate) fn contains(self, required: Self) -> bool {
        (!required.geometry || self.geometry)
            && (!required.material || self.material)
            && (!required.animation || self.animation)
    }

    pub(crate) fn missing_from(self, satisfied: Self) -> Self {
        Self {
            geometry: self.geometry && !satisfied.geometry,
            material: self.material && !satisfied.material,
            animation: self.animation && !satisfied.animation,
        }
    }

    pub(crate) fn is_empty(self) -> bool {
        !self.geometry && !self.material && !self.animation
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScenePayloadDescriptor {
    pub(crate) address: SceneCacheAddress,
    pub(crate) prim_path: String,
    pub(crate) material: Option<SceneCacheBlobRef>,
    pub(crate) animation: Option<SceneCacheBlobRef>,
}

#[derive(Clone, Debug, Default, Resource)]
pub(crate) struct ScenePayloadCatalog {
    pub(crate) scene_id: Option<SceneId>,
    pub(crate) generation: Option<u64>,
    pub(super) by_key: HashMap<ScenePayloadKey, Vec<ScenePayloadDescriptor>>,
}

impl ScenePayloadCatalog {
    pub(crate) fn from_presentation(presentation: &SceneCachePresentation) -> Result<Self, String> {
        let mut catalog = Self {
            scene_id: Some(presentation.scene_id),
            generation: Some(presentation.generation),
            by_key: HashMap::new(),
        };
        for entry in &presentation.entries {
            if entry.address.scene_id != presentation.scene_id {
                return Err("Scene payload address has a different SceneId".to_owned());
            }
            let SceneCacheEntryKind::OwnedPrim { prim_path } = &entry.kind else {
                continue;
            };
            if !entry.cacheable {
                continue;
            }
            let (Some(geometry), Some(blob_hash)) = (entry.geometry.as_ref(), entry.content_hash)
            else {
                continue;
            };
            validate_blob_ref(geometry, "geometry")?;
            if let Some(material) = entry.material.as_ref() {
                validate_blob_ref(material, "material")?;
            }
            if let Some(animation) = entry.animation.as_ref() {
                validate_blob_ref(animation, "animation")?;
            }
            let key = ScenePayloadKey {
                scene_id: presentation.scene_id,
                blob_hash,
            };
            catalog
                .by_key
                .entry(key)
                .or_default()
                .push(ScenePayloadDescriptor {
                    address: entry.address.clone(),
                    prim_path: prim_path.clone(),
                    material: entry.material.clone(),
                    animation: entry.animation.clone(),
                });
        }
        Ok(catalog)
    }

    pub(crate) fn descriptors_for(&self, key: ScenePayloadKey) -> &[ScenePayloadDescriptor] {
        self.by_key.get(&key).map(Vec::as_slice).unwrap_or(&[])
    }

    pub(crate) fn has_payload(&self, key: ScenePayloadKey, mask: PayloadLoadMask) -> bool {
        self.descriptors_for(key).iter().any(|descriptor| {
            (!mask.material || descriptor.material.is_some())
                && (!mask.animation || descriptor.animation.is_some())
        })
    }
}

fn validate_blob_ref(reference: &SceneCacheBlobRef, kind: &str) -> Result<(), String> {
    if reference.blob_id.0.trim().is_empty() {
        return Err(format!("Scene {kind} payload has an empty blob id"));
    }
    if reference.byte_size == 0 {
        return Err(format!("Scene {kind} payload has zero byte size"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/residency/catalog_inline_tests.rs"]
mod tests;
