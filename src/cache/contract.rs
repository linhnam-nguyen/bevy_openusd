//! Neutral persistent Scene-cache contracts and immutable Project lookup.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use usd_model::{BlobId, Bounds3, HashDigest, TransformSignature};
use usd_project::{ModelId, SceneId, SceneMemberId, ScenePlacementTransform};

/// Stable Project content target used in cache identities and source-closure hashing.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum ProjectCacheTarget {
    ProjectRoot,
    Scene { id: String },
    Model { id: String },
}

impl ProjectCacheTarget {
    pub(crate) fn key(&self) -> String {
        match self {
            Self::ProjectRoot => "project".to_owned(),
            Self::Scene { id } => format!("scene:{}", id),
            Self::Model { id } => format!("model:{}", id),
        }
    }
}

pub(crate) const SCENE_CACHE_DESCRIPTOR_SCHEMA_VERSION: u16 = 3;
pub(crate) const SCENE_CACHE_INDEX_SCHEMA_VERSION: u16 = 3;
pub(crate) const SCENE_SPATIAL_INDEX_SCHEMA_VERSION: u16 = 1;
pub(crate) const PROJECT_CACHE_INDEX_SCHEMA_VERSION: u16 = 2;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SceneSourceStamp {
    GitRevision {
        revision: String,
    },
    ManagedGeneration {
        generation: u64,
    },
    ExternalFile {
        modified_unix_nanos: u64,
        byte_len: u64,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SceneCacheState {
    Empty,
    Building,
    Partial,
    Ready,
    Stale,
    Failed,
    Retired,
    FallbackRequired,
}

impl SceneCacheState {
    pub(crate) fn lifecycle_state(self) -> super::sync::CacheLifecycleState {
        match self {
            Self::Empty => super::sync::CacheLifecycleState::Missing,
            Self::Building => super::sync::CacheLifecycleState::Building,
            Self::Ready => super::sync::CacheLifecycleState::Ready,
            Self::Partial => super::sync::CacheLifecycleState::Stale,
            Self::Stale => super::sync::CacheLifecycleState::Stale,
            Self::Failed | Self::FallbackRequired => super::sync::CacheLifecycleState::Failed,
            Self::Retired => super::sync::CacheLifecycleState::Retired,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct SceneCacheDescriptorV3 {
    pub(crate) schema_version: u16,
    pub(crate) scene_id: SceneId,
    pub(crate) generation: u64,
    pub(crate) source_stamp: SceneSourceStamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source_content_hash: Option<HashDigest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) revision_identity: Option<HashDigest>,
    pub(crate) config_hash: HashDigest,
    pub(crate) state: SceneCacheState,
    pub(crate) prim_count: u64,
    #[serde(default)]
    pub(crate) prim_count_ready: bool,
    pub(crate) cacheable_count: u64,
    pub(crate) estimated_cpu_bytes: u64,
    pub(crate) estimated_gpu_bytes: u64,
    pub(crate) index_digest: HashDigest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) spatial_digest: Option<HashDigest>,
}

impl SceneCacheDescriptorV3 {
    pub(crate) fn invalidated(scene_id: SceneId, generation: u64, config_hash: HashDigest) -> Self {
        Self {
            schema_version: SCENE_CACHE_DESCRIPTOR_SCHEMA_VERSION,
            scene_id,
            generation,
            source_stamp: SceneSourceStamp::ManagedGeneration { generation },
            source_content_hash: None,
            revision_identity: None,
            config_hash,
            state: SceneCacheState::Building,
            prim_count: 0,
            prim_count_ready: false,
            cacheable_count: 0,
            estimated_cpu_bytes: 0,
            estimated_gpu_bytes: 0,
            index_digest: HashDigest::new([0; HashDigest::BYTE_LEN]),
            spatial_digest: None,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == SCENE_CACHE_DESCRIPTOR_SCHEMA_VERSION,
            "unsupported Scene cache descriptor schema version {}",
            self.schema_version
        );
        ensure!(
            self.cacheable_count <= self.prim_count,
            "Scene cacheable prim count exceeds total prim count"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) enum SceneCacheOccurrence {
    PrimPath(String),
    Member(SceneMemberId),
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct SceneCacheAddress {
    pub(crate) scene_id: SceneId,
    pub(crate) occurrence: SceneCacheOccurrence,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) enum CachedTransform {
    Prim(TransformSignature),
    Placement(ScenePlacementTransform),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct SceneCacheBlobRef {
    pub(crate) blob_id: BlobId,
    pub(crate) byte_size: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) enum SceneCacheEntryKind {
    OwnedPrim {
        prim_path: String,
    },
    ChildScene {
        scene_id: SceneId,
        member_id: SceneMemberId,
    },
    ChildModel {
        model_id: ModelId,
        member_id: SceneMemberId,
    },
}

/// Direct logical child count for a cached hierarchy node.
///
/// Unknown child counts (unloaded payloads or unresolved nested members) stay
/// distinct from a confirmed leaf (`Known(0)`).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", content = "count", rename_all = "snake_case")]
pub(crate) enum HierarchyChildCount {
    Known(u32),
    Unknown,
}

impl Default for HierarchyChildCount {
    fn default() -> Self {
        Self::Known(0)
    }
}

impl HierarchyChildCount {
    pub(crate) fn has_children(self) -> bool {
        matches!(self, Self::Unknown) || matches!(self, Self::Known(count) if count > 0)
    }
}

const fn default_authored_visible() -> bool {
    true
}

/// Persisted logical hierarchy metadata for one Scene cache entry.
///
/// Authored visibility is stored separately from transient runtime overrides,
/// and authored display names remain distinct from semantic identity keys.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CachedHierarchyMetadata {
    #[serde(default)]
    pub(crate) metadata_populated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) type_name: Option<String>,
    #[serde(default)]
    pub(crate) child_count: HierarchyChildCount,
    #[serde(default = "default_authored_visible")]
    pub(crate) authored_visible: bool,
    #[serde(default)]
    pub(crate) transparent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) target_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) target_id: Option<String>,
    /// A flat snapshot of descendants in this member's actual composed occurrence.
    /// None means the branch has not been qualified; never infer a leaf from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) composed_descendants: Option<Vec<CachedHierarchyDescendant>>,
}

/// One composed descendant's metadata, without duplicate render resources.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CachedHierarchyDescendant {
    pub(crate) prim_path: String,
    pub(crate) metadata: CachedHierarchyMetadata,
}

impl Default for CachedHierarchyMetadata {
    fn default() -> Self {
        Self {
            metadata_populated: false,
            display_name: None,
            type_name: None,
            child_count: HierarchyChildCount::Known(0),
            authored_visible: true,
            transparent: false,
            target_kind: None,
            target_id: None,
            composed_descendants: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct SceneCacheEntry {
    pub(crate) address: SceneCacheAddress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) parent: Option<u32>,
    pub(crate) transform: CachedTransform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) bounds: Option<Bounds3>,
    pub(crate) cacheable: bool,
    pub(crate) bim_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) geometry: Option<SceneCacheBlobRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) material: Option<SceneCacheBlobRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) animation: Option<SceneCacheBlobRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) semantic_key: Option<String>,
    pub(crate) kind: SceneCacheEntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) content_hash: Option<HashDigest>,
    #[serde(default)]
    pub(crate) hierarchy: CachedHierarchyMetadata,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct SceneCacheIndex {
    pub(crate) schema_version: u16,
    pub(crate) scene_id: SceneId,
    pub(crate) generation: u64,
    pub(crate) entries: Vec<SceneCacheEntry>,
}

/// Stable cache-first activation snapshot. The descriptor is read before the
/// index so callers can publish lightweight metadata without opening or
/// projecting the canonical Stage. OpenUSD remains the authority for any
/// source revalidation and for all heavy projection payloads.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SceneCacheActivation {
    pub(crate) descriptor: SceneCacheDescriptorV3,
    pub(crate) index: SceneCacheIndex,
}

impl SceneCacheIndex {
    pub(crate) fn validate(&self, scene_id: SceneId, generation: u64) -> Result<()> {
        ensure!(
            self.schema_version == SCENE_CACHE_INDEX_SCHEMA_VERSION,
            "unsupported Scene cache index schema version {}",
            self.schema_version
        );
        ensure!(
            self.scene_id == scene_id,
            "Scene cache index SceneId mismatch"
        );
        ensure!(
            self.generation == generation,
            "Scene cache index generation mismatch"
        );
        ensure!(
            self.entries
                .windows(2)
                .all(|pair| pair[0].address < pair[1].address),
            "Scene cache index addresses must be unique and strictly ordered"
        );
        for (index, entry) in self.entries.iter().enumerate() {
            ensure!(
                entry.address.scene_id == self.scene_id,
                "Scene cache entry belongs to a different Scene"
            );
            if let Some(parent) = entry.parent {
                ensure!(
                    (parent as usize) < self.entries.len() && parent as usize != index,
                    "Scene cache parent index is invalid"
                );
            }
            if let Some(geometry) = &entry.geometry {
                ensure!(entry.cacheable, "Scene cache geometry must be cacheable");
                let digest = HashDigest::from_hex(&geometry.blob_id.0)
                    .map_err(|_| anyhow::anyhow!("Scene cache geometry blob id is invalid"))?;
                ensure!(
                    entry.content_hash == Some(digest),
                    "Scene cache content hash must match geometry blob"
                );
            }
        }
        // Verify that parent links form a forest (no cycles).
        let mut visit_state = vec![0_u8; self.entries.len()];
        for start in 0..self.entries.len() {
            if visit_state[start] != 0 {
                continue;
            }
            let mut chain = Vec::new();
            let mut current = Some(start);
            while let Some(idx) = current {
                if visit_state[idx] == 1 {
                    anyhow::bail!("Scene cache parent hierarchy contains a cycle");
                }
                if visit_state[idx] == 2 {
                    break;
                }
                visit_state[idx] = 1;
                chain.push(idx);
                current = self.entries[idx].parent.map(|parent| parent as usize);
            }
            for idx in chain {
                visit_state[idx] = 2;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct SceneSpatialEntry {
    pub(crate) address: SceneCacheAddress,
    pub(crate) bounds: Bounds3,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct SceneSpatialIndex {
    pub(crate) schema_version: u16,
    pub(crate) scene_id: SceneId,
    pub(crate) generation: u64,
    pub(crate) entries: Vec<SceneSpatialEntry>,
}

impl SceneSpatialIndex {
    pub(crate) fn validate(&self, scene_id: SceneId, generation: u64) -> Result<()> {
        ensure!(
            self.schema_version == SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
            "unsupported Scene spatial index schema version {}",
            self.schema_version
        );
        ensure!(
            self.scene_id == scene_id,
            "Scene spatial index SceneId mismatch"
        );
        ensure!(
            self.generation == generation,
            "Scene spatial index generation mismatch"
        );
        ensure!(
            self.entries
                .windows(2)
                .all(|pair| pair[0].address < pair[1].address),
            "Scene spatial index addresses must be unique and strictly ordered"
        );
        ensure!(
            self.entries
                .iter()
                .all(|entry| entry.address.scene_id == self.scene_id),
            "Scene spatial entry belongs to a different Scene"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum CacheObjectRef {
    OwnedPrim {
        prim_path: String,
        geometry: Option<SceneCacheBlobRef>,
        material: Option<SceneCacheBlobRef>,
        animation: Option<SceneCacheBlobRef>,
        semantic_key: Option<String>,
        content_hash: Option<HashDigest>,
    },
    ChildScene {
        scene_id: SceneId,
        member_id: SceneMemberId,
    },
    ChildModel {
        model_id: ModelId,
        member_id: SceneMemberId,
    },
}

pub(crate) use super::lookup::{ProjectCacheLookup, SceneObjectKey};
