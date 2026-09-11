//! Generation-safe publication of Scene-owned derived prim-count metadata.

use anyhow::{Result, bail, ensure};
use usd_model::HashDigest;
use usd_project::SceneId;

use super::{SceneCacheDescriptorV3, SceneCacheStore};
use crate::project::cache_contract::{
    SCENE_CACHE_INDEX_SCHEMA_VERSION, SceneCacheIndex, SceneCacheState,
    SCENE_SPATIAL_INDEX_SCHEMA_VERSION, SceneSpatialIndex,
};

#[derive(Debug)]
pub(crate) enum PrimCountPublication {
    Published(SceneCacheDescriptorV3),
    RetryableCurrent(SceneCacheDescriptorV3),
    Stale,
}

pub(crate) fn publish_prim_count_if_current(
    store: &SceneCacheStore,
    scene_id: SceneId,
    expected: Option<&SceneCacheDescriptorV3>,
    config_hash: HashDigest,
    prim_count: u64,
) -> Result<PrimCountPublication> {
    store.with_generation_lock(scene_id, || {
        let current = store.load_descriptor(scene_id)?;
        let mut descriptor = match (expected, current) {
            (None, None) => {
                let mut descriptor = SceneCacheDescriptorV3::invalidated(
                    scene_id,
                    1,
                    config_hash,
                );
                descriptor.state = SceneCacheState::Partial;
                descriptor
            }
            (None, Some(current)) => return Ok(PrimCountPublication::RetryableCurrent(current)),
            (Some(expected), Some(current)) if compatible_identity(scene_id, expected, &current) => {
                current
            }
            (Some(_), Some(_)) | (Some(_), None) => {
                return Ok(PrimCountPublication::Stale);
            }
        };
        let bootstrap = expected.is_none() && descriptor.generation == 1;
        let allow_absent_artifacts = bootstrap || allows_absent_artifacts(descriptor.state);
        let index = match store.load_index(scene_id)? {
            Some(index) => index,
            None if allow_absent_artifacts && index_digest_is_empty(&descriptor) => {
                empty_index(scene_id, descriptor.generation)
            }
            None => bail!(
                "Scene cache index is absent for current generation {}",
                descriptor.generation
            ),
        };
        ensure!(index.scene_id == scene_id, "Scene cache index owner changed");
        ensure!(index.generation == descriptor.generation, "Scene cache index generation changed");
        let spatial = match store.load_spatial(scene_id)? {
            Some(spatial) => spatial,
            None if allow_absent_artifacts && descriptor.spatial_digest.is_none() => {
                spatial_index(&index)
            }
            None => bail!(
                "Scene spatial index is absent for current generation {}",
                descriptor.generation
            ),
        };
        ensure!(spatial.scene_id == scene_id, "Scene spatial index owner changed");
        ensure!(spatial.generation == descriptor.generation, "Scene spatial index generation changed");
        ensure!(descriptor.cacheable_count <= prim_count, "Scene cache prim count is below indexed entries");
        descriptor.prim_count = prim_count;
        descriptor.prim_count_ready = true;
        Ok(PrimCountPublication::Published(
            store.publish_generation(&descriptor, &index, &spatial)?,
        ))
    })
}

fn compatible_identity(
    scene_id: SceneId,
    expected: &SceneCacheDescriptorV3,
    current: &SceneCacheDescriptorV3,
) -> bool {
    expected.scene_id == scene_id
        && current.scene_id == scene_id
        && expected.schema_version == current.schema_version
        && expected.generation == current.generation
        && expected.config_hash == current.config_hash
        && expected.source_stamp == current.source_stamp
        && expected.source_content_hash == current.source_content_hash
}

fn allows_absent_artifacts(state: SceneCacheState) -> bool {
    matches!(
        state,
        SceneCacheState::Empty | SceneCacheState::Building | SceneCacheState::FallbackRequired
    )
}

fn index_digest_is_empty(descriptor: &SceneCacheDescriptorV3) -> bool {
    descriptor.index_digest == HashDigest::new([0; HashDigest::BYTE_LEN])
}

fn empty_index(scene_id: SceneId, generation: u64) -> SceneCacheIndex {
    SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation,
        entries: Vec::new(),
    }
}

fn spatial_index(index: &SceneCacheIndex) -> SceneSpatialIndex {
    SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id: index.scene_id,
        generation: index.generation,
        entries: index
            .entries
            .iter()
            .filter_map(|entry| entry.bounds.map(|bounds| crate::project::cache_contract::SceneSpatialEntry {
                address: entry.address.clone(),
                bounds,
            }))
            .collect(),
    }
}

#[cfg(test)]
#[path = "cache_scene_prim_count_tests.rs"]
mod tests;
