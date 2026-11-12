//! Build self-render delivery payloads from the authoritative semantic state.
//!
//! This adapter is intentionally application-owned. The protocol sees only a
//! deterministic runtime manifest and content-addressed bytes; it never sees
//! the local project path, Bevy handles, or semantic property values.

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use usd_model::{Bounds3, EntitySnapshot, SemanticSnapshot, TransformSignature};
use viewport_protocol::{
    RuntimeBlobReference, RuntimeManifest, RuntimePayloadKind, RuntimeProfile,
};

use super::blob_store::BlobStore;
use super::runtime_payload::{PreparedRuntimePayloads, RuntimeMaterialBlob, RuntimeTextureBlob};

pub(crate) const RUNTIME_HIERARCHY_VERSION: u16 = 3;
pub(crate) const RUNTIME_MESH_VERSION: u16 = 1;

/// A complete server-owned manifest and the verified bytes it references.
#[derive(Debug)]
pub(crate) struct RuntimeDeliveryBundle {
    pub(crate) manifest: RuntimeManifest,
    pub(crate) blobs: Vec<(String, Vec<u8>)>,
}

/// Build a deterministic runtime bundle from one enriched semantic snapshot.
///
/// The hierarchy blob is derived from stable runtime information only. Mesh
/// blobs are reused from the existing content-addressed store and verified by
/// [`BlobStore::get`] before they are included in the bundle.
pub(crate) fn build_runtime_delivery(
    store: &impl BlobStore,
    snapshot: &SemanticSnapshot,
    profile: RuntimeProfile,
) -> Result<RuntimeDeliveryBundle> {
    build_runtime_delivery_with_payloads(
        store,
        snapshot,
        profile,
        &PreparedRuntimePayloads::default(),
    )
}

/// Build a runtime bundle with payloads prepared from the same authoritative
/// projection. The old helper remains useful for mesh-only fallback tests.
pub(crate) fn build_runtime_delivery_with_payloads(
    store: &impl BlobStore,
    snapshot: &SemanticSnapshot,
    profile: RuntimeProfile,
    prepared: &PreparedRuntimePayloads,
) -> Result<RuntimeDeliveryBundle> {
    let mut entities = snapshot.entities.values().collect::<Vec<_>>();
    entities.sort_by(|left, right| left.key.cmp(&right.key));

    let mut mesh_ids = BTreeSet::new();
    let hierarchy_entities = entities
        .iter()
        .map(|entity| {
            hierarchy_entity(
                entity,
                &mut mesh_ids,
                prepared.material_by_entity.get(&entity.prim_path).cloned(),
            )
        })
        .collect::<Vec<_>>();
    let hierarchy = RuntimeHierarchyBlob {
        version: RUNTIME_HIERARCHY_VERSION,
        revision: snapshot.snapshot_id.0.clone(),
        entities: hierarchy_entities,
    };
    let hierarchy_bytes =
        serde_json::to_vec(&hierarchy).context("encode runtime hierarchy blob")?;
    let hierarchy_blob_id = store
        .put(&hierarchy_bytes)
        .context("store runtime hierarchy blob")?;

    let mut blobs = vec![(hierarchy_blob_id.0.clone(), hierarchy_bytes)];
    let mut mesh_references = Vec::with_capacity(mesh_ids.len());
    for blob_id in mesh_ids {
        let model_blob_id = usd_model::BlobId(blob_id.clone());
        let Some(bytes) = store
            .get(&model_blob_id)
            .with_context(|| format!("read runtime mesh blob {blob_id}"))?
        else {
            bail!("runtime mesh blob {blob_id} is missing from the BlobStore");
        };
        let byte_size = bytes.len() as u64;
        blobs.push((blob_id.clone(), bytes));
        mesh_references.push(RuntimeBlobReference {
            blob_id,
            payload_kind: RuntimePayloadKind::Mesh,
            payload_version: RUNTIME_MESH_VERSION,
            byte_size,
        });
    }

    let mut material_references = Vec::with_capacity(prepared.materials.len());
    for payload in &prepared.materials {
        let material_id = payload.blob_id.0.clone();
        let bytes = store
            .get(&payload.blob_id)
            .with_context(|| format!("read runtime material blob {material_id}"))?
            .ok_or_else(|| anyhow::anyhow!("runtime material blob {material_id} is missing"))?;
        let descriptor: RuntimeMaterialBlob = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode runtime material blob {material_id}"))?;
        descriptor.validate()?;
        let byte_size = bytes.len() as u64;
        blobs.push((material_id.clone(), bytes));
        material_references.push(RuntimeBlobReference {
            blob_id: material_id,
            payload_kind: RuntimePayloadKind::Material,
            payload_version: descriptor.version,
            byte_size,
        });
    }

    let mut texture_references = Vec::with_capacity(prepared.textures.len());
    for payload in &prepared.textures {
        let texture_id = payload.blob_id.0.clone();
        let bytes = store
            .get(&payload.blob_id)
            .with_context(|| format!("read runtime texture blob {texture_id}"))?
            .ok_or_else(|| anyhow::anyhow!("runtime texture blob {texture_id} is missing"))?;
        let descriptor: RuntimeTextureBlob = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode runtime texture blob {texture_id}"))?;
        descriptor.validate()?;
        let byte_size = bytes.len() as u64;
        blobs.push((texture_id.clone(), bytes));
        texture_references.push(RuntimeBlobReference {
            blob_id: texture_id,
            payload_kind: RuntimePayloadKind::Texture,
            payload_version: descriptor.version,
            byte_size,
        });
    }

    Ok(RuntimeDeliveryBundle {
        manifest: RuntimeManifest {
            revision: snapshot.snapshot_id.0.clone(),
            profile,
            hierarchy: RuntimeBlobReference {
                blob_id: hierarchy_blob_id.0,
                payload_kind: RuntimePayloadKind::Hierarchy,
                payload_version: RUNTIME_HIERARCHY_VERSION,
                byte_size: blobs[0].1.len() as u64,
            },
            meshes: mesh_references,
            materials: material_references,
            textures: texture_references,
        },
        blobs,
    })
}

fn hierarchy_entity(
    entity: &EntitySnapshot,
    mesh_ids: &mut BTreeSet<String>,
    material_blob_id: Option<String>,
) -> RuntimeHierarchyEntity {
    let geometry = entity.geometry.as_ref().and_then(|geometry| {
        let blob_id = geometry.render_blob.as_ref()?.0.clone();
        mesh_ids.insert(blob_id.clone());
        Some(RuntimeHierarchyGeometry {
            blob_id,
            index_count: geometry.index_count,
            vertex_count: geometry.vertex_count,
            local_bounds: geometry.local_bounds,
        })
    });

    RuntimeHierarchyEntity {
        entity_key: entity.key.0.clone(),
        prim_path: entity.prim_path.clone(),
        display_name: entity.semantic.display_name.clone(),
        transform: entity.transform.clone(),
        geometry,
        material_blob_id,
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct RuntimeHierarchyBlob {
    pub(crate) version: u16,
    pub(crate) revision: String,
    pub(crate) entities: Vec<RuntimeHierarchyEntity>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct RuntimeHierarchyEntity {
    pub(crate) entity_key: String,
    pub(crate) prim_path: String,
    #[serde(default)]
    pub(crate) display_name: Option<String>,
    pub(crate) transform: TransformSignature,
    pub(crate) geometry: Option<RuntimeHierarchyGeometry>,
    #[serde(default)]
    pub(crate) material_blob_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct RuntimeHierarchyGeometry {
    pub(crate) blob_id: String,
    pub(crate) vertex_count: u32,
    pub(crate) index_count: u32,
    pub(crate) local_bounds: Bounds3,
}

/// Return the bundle parts for the atomic application-interface publish.
pub(crate) fn into_delivery_parts(
    bundle: RuntimeDeliveryBundle,
) -> (RuntimeManifest, Vec<(String, Vec<u8>)>) {
    (bundle.manifest, bundle.blobs)
}

#[cfg(test)]
#[path = "../../tests/unit/project/runtime_delivery_tests.rs"]
mod tests;
