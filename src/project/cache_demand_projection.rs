//! Exact-path Scene cache filling for uncached selection demand.

use std::path::Path;

use anyhow::{Result, ensure};
use openusd::sdf;
use usd_model::HashDigest;
use usd_project::SceneId;

use super::blob_store::{BlobStore, get_mesh, prepare_mesh_payload};
use super::cache::{ProjectCacheTarget, SceneCacheStore, target_content_hash};
use super::cache_contract::{SceneCacheBlobRef, SceneCacheDescriptorV3, SceneCacheEntryKind};
use super::cache_warm_runtime;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OwnedPrimPathResolution {
    Ready(String),
    Waiting,
    Rejected,
}

#[derive(Debug)]
pub(crate) struct TargetedSceneRepair {
    pub(crate) expected_descriptor: SceneCacheDescriptorV3,
    pub(crate) payloads: Vec<usd_bevy::TargetedRenderPayload>,
}

/// Resolve a residency payload through its owning Scene cache.
///
/// The SceneId is part of the payload key and therefore cannot be replaced by
/// the currently visible composed Scene. Only an explicit OwnedPrim cache row
/// is eligible; child-reference rows and composed-only paths do not acquire a
/// synthetic owner here.
pub(crate) fn owned_prim_path_for_payload(
    project_root: &Path,
    scene_id: SceneId,
    blob_hash: HashDigest,
) -> Result<OwnedPrimPathResolution> {
    let store = SceneCacheStore::new(project_root);
    let Some(descriptor) = store.load_descriptor(scene_id)? else {
        return Ok(OwnedPrimPathResolution::Rejected);
    };
    match descriptor.state {
        super::cache_contract::SceneCacheState::Building
        | super::cache_contract::SceneCacheState::Empty => {
            return Ok(OwnedPrimPathResolution::Waiting);
        }
        super::cache_contract::SceneCacheState::FallbackRequired => {
            return Ok(OwnedPrimPathResolution::Rejected);
        }
        super::cache_contract::SceneCacheState::Partial
        | super::cache_contract::SceneCacheState::Ready => {}
    }
    let Some(activation) = store.load_activation(scene_id)? else {
        return Ok(OwnedPrimPathResolution::Waiting);
    };
    Ok(activation
        .index
        .entries
        .iter()
        .find_map(|entry| {
            (entry.address.scene_id == scene_id
                && entry.content_hash == Some(blob_hash)
                && matches!(&entry.kind, SceneCacheEntryKind::OwnedPrim { .. }))
            .then(|| match &entry.kind {
                SceneCacheEntryKind::OwnedPrim { prim_path } => prim_path.clone(),
                _ => unreachable!("OwnedPrim match must produce an owned path"),
            })
        })
        .map_or(
            OwnedPrimPathResolution::Rejected,
            OwnedPrimPathResolution::Ready,
        ))
}

/// Extract only one requested render payload from the canonical LiveStage.
///
/// Cache activation is read first and the owner path is revalidated before
/// extraction. Persistence is deliberately owned by the residency worker.
pub(crate) fn extract_scene_payloads_for_repair(
    project_root: &Path,
    scene_id: SceneId,
    blob_hash: HashDigest,
    live: &usd_bevy::LiveStage,
    path: &str,
) -> Result<Option<TargetedSceneRepair>> {
    let store = SceneCacheStore::new(project_root);
    let Some(activation) = store.load_activation(scene_id)? else {
        return Ok(None);
    };
    ensure!(
        activation.index.entries.iter().any(|entry| {
            entry.address.scene_id == scene_id
                && entry.content_hash == Some(blob_hash)
                && matches!(&entry.kind, SceneCacheEntryKind::OwnedPrim { prim_path } if prim_path == path)
        }),
        "targeted Scene extraction rejects unknown or composed-only path {path}"
    );
    let sdf_path = sdf::path(path)?;
    if !live.stage.prim(sdf_path).is_loaded()? {
        live.load_payload(path);
    }
    let requested = [path];
    let payloads = usd_bevy::extract_render_payloads_for_paths(&live.stage, &requested)?;
    Ok(Some(TargetedSceneRepair {
        expected_descriptor: activation.descriptor,
        payloads,
    }))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TargetedPersistenceOutcome {
    Published {
        lookup_repaired: bool,
        published_descriptor: SceneCacheDescriptorV3,
    },
    LookupRepaired,
    LookupPending,
    ScenePayloadMissing,
    ScenePayloadNotOwned,
    CasLost,
    Waiting,
}

/// Validate a cached lookup through the owning Scene cache before publishing
/// the Project lookup. Storage identity and payload readability stay here so
/// residency only routes the typed result.
pub(crate) fn persist_lookup_repair(
    project_root: &Path,
    scene_id: SceneId,
    expected: &SceneCacheDescriptorV3,
    path: &str,
    blob_hash: HashDigest,
) -> Result<TargetedPersistenceOutcome> {
    let store = SceneCacheStore::new(project_root);
    let Some(activation) = store.load_activation(scene_id)? else {
        return Ok(TargetedPersistenceOutcome::Waiting);
    };
    if activation.descriptor != *expected {
        return Ok(TargetedPersistenceOutcome::CasLost);
    }
    let Some(entry) = activation.index.entries.iter().find(|entry| {
        entry.address.scene_id == scene_id
            && entry.content_hash == Some(blob_hash)
            && matches!(&entry.kind, SceneCacheEntryKind::OwnedPrim { prim_path } if prim_path == path)
    }) else {
        return Ok(TargetedPersistenceOutcome::ScenePayloadNotOwned);
    };
    let Some(geometry) = entry.geometry.as_ref() else {
        return Ok(TargetedPersistenceOutcome::ScenePayloadMissing);
    };
    let object_store = match store.object_store(scene_id) {
        Ok(object_store) => object_store,
        Err(_) => return Ok(TargetedPersistenceOutcome::ScenePayloadMissing),
    };
    if !matches!(get_mesh(&object_store, &geometry.blob_id), Ok(Some(_))) {
        return Ok(TargetedPersistenceOutcome::ScenePayloadMissing);
    }
    Ok(
        if cache_warm_runtime::publish_current_project_cache_lookup(project_root).is_ok() {
            TargetedPersistenceOutcome::LookupRepaired
        } else {
            TargetedPersistenceOutcome::LookupPending
        },
    )
}

/// Prepare, persist, and publish an extracted repair off the Bevy owner thread.
pub(crate) fn persist_scene_payloads_for_repair(
    project_root: &Path,
    scene_id: SceneId,
    expected: &SceneCacheDescriptorV3,
    payloads: Vec<usd_bevy::TargetedRenderPayload>,
) -> Result<TargetedPersistenceOutcome> {
    let store = SceneCacheStore::new(project_root);
    let Some(activation) = store.load_activation(scene_id)? else {
        return Ok(TargetedPersistenceOutcome::Waiting);
    };
    if activation.descriptor != *expected {
        return Ok(TargetedPersistenceOutcome::CasLost);
    }
    verify_source_identity(project_root, scene_id, expected.source_content_hash)?;
    let mut index = activation.index;
    let object_store = store.object_store(scene_id)?;
    for payload in payloads {
        let Some(entry) = index.entries.iter_mut().find(|entry| {
            matches!(&entry.kind, SceneCacheEntryKind::OwnedPrim { prim_path } if prim_path == &payload.path)
        }) else {
            ensure!(false, "targeted Scene persistence cannot synthesize an OwnedPrim");
            unreachable!();
        };
        let prepared = prepare_mesh_payload(&payload.mesh)?;
        let stored = object_store.put(&prepared.bytes)?;
        ensure!(
            stored == prepared.blob_id,
            "Scene targeted geometry digest mismatch"
        );
        entry.cacheable = true;
        entry.geometry = Some(SceneCacheBlobRef {
            blob_id: stored.clone(),
            byte_size: prepared.bytes.len() as u64,
        });
        entry.content_hash = Some(HashDigest::from_hex(&stored.0)?);
        if entry.bounds.is_none() {
            entry.bounds = payload.local_bounds;
        }
    }
    verify_source_identity(project_root, scene_id, expected.source_content_hash)?;
    index
        .entries
        .sort_by(|left, right| left.address.cmp(&right.address));
    cache_warm_runtime::assign_parent_indexes(&mut index.entries)?;
    let spatial = cache_warm_runtime::build_spatial_index(&index);
    let mut descriptor = expected.clone();
    descriptor.prim_count = index.entries.len() as u64;
    descriptor.cacheable_count =
        index.entries.iter().filter(|entry| entry.cacheable).count() as u64;
    let payload_bytes = index
        .entries
        .iter()
        .filter_map(|entry| entry.geometry.as_ref())
        .map(|geometry| geometry.byte_size)
        .sum();
    descriptor.estimated_cpu_bytes = payload_bytes;
    descriptor.estimated_gpu_bytes = payload_bytes;
    let Some(published_descriptor) =
        store.publish_generation_if_current(expected, &descriptor, &index, &spatial)?
    else {
        return Ok(TargetedPersistenceOutcome::CasLost);
    };
    let lookup_repaired =
        cache_warm_runtime::publish_current_project_cache_lookup(project_root).is_ok();
    Ok(TargetedPersistenceOutcome::Published {
        lookup_repaired,
        published_descriptor,
    })
}

fn normalized_requested_paths(requested_paths: &[String]) -> Result<Vec<String>> {
    let mut normalized = requested_paths
        .iter()
        .map(|path| usd_bevy::validate_prim_path(path))
        .collect::<Result<Vec<_>>>()?;
    normalized.retain(|path| path != "/");
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn verify_source_identity(
    project_root: &Path,
    scene_id: SceneId,
    expected: Option<HashDigest>,
) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let target = ProjectCacheTarget::Scene {
        id: scene_id.to_string(),
    };
    ensure!(
        target_content_hash(project_root, &target)? == expected,
        "Scene source changed during targeted cache fill"
    );
    Ok(())
}

#[cfg(test)]
#[path = "cache_demand_projection_tests.rs"]
mod tests;
