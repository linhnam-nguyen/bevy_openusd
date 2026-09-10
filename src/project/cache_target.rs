//! Neutral persistent Scene-cache contracts and immutable Project lookup.

use std::collections::{HashMap, HashSet};

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
pub(crate) const SCENE_CACHE_INDEX_SCHEMA_VERSION: u16 = 2;
pub(crate) const SCENE_SPATIAL_INDEX_SCHEMA_VERSION: u16 = 1;
pub(crate) const PROJECT_CACHE_INDEX_SCHEMA_VERSION: u16 = 2;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SceneSourceStamp {
    GitRevision { revision: String },
    ManagedGeneration { generation: u64 },
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
    FallbackRequired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct SceneCacheDescriptorV3 {
    pub(crate) schema_version: u16,
    pub(crate) scene_id: SceneId,
    pub(crate) generation: u64,
    pub(crate) source_stamp: SceneSourceStamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source_content_hash: Option<HashDigest>,
    pub(crate) config_hash: HashDigest,
    pub(crate) state: SceneCacheState,
    pub(crate) prim_count: u64,
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
            config_hash,
            state: SceneCacheState::Building,
            prim_count: 0,
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
    OwnedPrim { prim_path: String },
    ChildScene { scene_id: SceneId, member_id: SceneMemberId },
    ChildModel { model_id: ModelId, member_id: SceneMemberId },
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
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct SceneCacheIndex {
    pub(crate) schema_version: u16,
    pub(crate) scene_id: SceneId,
    pub(crate) generation: u64,
    pub(crate) entries: Vec<SceneCacheEntry>,
}

impl SceneCacheIndex {
    pub(crate) fn validate(&self, scene_id: SceneId, generation: u64) -> Result<()> {
        ensure!(
            self.schema_version == SCENE_CACHE_INDEX_SCHEMA_VERSION,
            "unsupported Scene cache index schema version {}",
            self.schema_version
        );
        ensure!(self.scene_id == scene_id, "Scene cache index SceneId mismatch");
        ensure!(self.generation == generation, "Scene cache index generation mismatch");
        ensure!(
            self.entries.windows(2).all(|pair| pair[0].address < pair[1].address),
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
        ensure!(self.scene_id == scene_id, "Scene spatial index SceneId mismatch");
        ensure!(self.generation == generation, "Scene spatial index generation mismatch");
        ensure!(
            self.entries.windows(2).all(|pair| pair[0].address < pair[1].address),
            "Scene spatial index addresses must be unique and strictly ordered"
        );
        ensure!(
            self.entries.iter().all(|entry| entry.address.scene_id == self.scene_id),
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
    ChildScene { scene_id: SceneId, member_id: SceneMemberId },
    ChildModel { model_id: ModelId, member_id: SceneMemberId },
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct SceneObjectKey {
    pub(crate) scene_id: SceneId,
    pub(crate) content_hash: HashDigest,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ProjectCacheLookup {
    by_address: HashMap<SceneCacheAddress, CacheObjectRef>,
    by_hash: HashMap<SceneObjectKey, Vec<SceneCacheAddress>>,
    generations: HashMap<SceneId, u64>,
}

impl ProjectCacheLookup {
    pub(crate) fn from_scene_indexes(indexes: &[SceneCacheIndex]) -> Result<Self> {
        let mut by_address = HashMap::new();
        let mut by_hash: HashMap<SceneObjectKey, Vec<SceneCacheAddress>> = HashMap::new();
        let mut generations = HashMap::new();
        let mut scenes = HashSet::new();
        for index in indexes {
            ensure!(
                scenes.insert(index.scene_id),
                "Project cache lookup cannot merge multiple generations of one Scene"
            );
            index.validate(index.scene_id, index.generation)?;
            generations.insert(index.scene_id, index.generation);
            for entry in &index.entries {
                let object = cache_object_ref(entry);
                ensure!(
                    by_address.insert(entry.address.clone(), object).is_none(),
                    "duplicate Project cache address"
                );
                if let Some(content_hash) = entry.content_hash {
                    by_hash
                        .entry(SceneObjectKey {
                            scene_id: index.scene_id,
                            content_hash,
                        })
                        .or_default()
                        .push(entry.address.clone());
                }
            }
        }
        for addresses in by_hash.values_mut() {
            addresses.sort();
        }
        Ok(Self {
            by_address,
            by_hash,
            generations,
        })
    }

    pub(crate) fn get(&self, address: &SceneCacheAddress) -> Option<&CacheObjectRef> {
        self.by_address.get(address)
    }

    pub(crate) fn addresses_for_hash(&self, key: SceneObjectKey) -> &[SceneCacheAddress] {
        self.by_hash.get(&key).map(Vec::as_slice).unwrap_or(&[])
    }

    pub(crate) fn persistent_rows(&self) -> Vec<(SceneCacheAddress, CacheObjectRef)> {
        let mut rows = self
            .by_address
            .iter()
            .map(|(address, object)| (address.clone(), object.clone()))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        rows
    }

    pub(crate) fn persistent_generations(&self) -> Vec<(SceneId, u64)> {
        let mut generations = self
            .generations
            .iter()
            .map(|(scene_id, generation)| (*scene_id, *generation))
            .collect::<Vec<_>>();
        generations.sort_by_key(|(scene_id, _)| *scene_id);
        generations
    }
}

fn cache_object_ref(entry: &SceneCacheEntry) -> CacheObjectRef {
    match &entry.kind {
        SceneCacheEntryKind::OwnedPrim { prim_path } => CacheObjectRef::OwnedPrim {
            prim_path: prim_path.clone(),
            geometry: entry.geometry.clone(),
            material: entry.material.clone(),
            animation: entry.animation.clone(),
            semantic_key: entry.semantic_key.clone(),
            content_hash: entry.content_hash,
        },
        SceneCacheEntryKind::ChildScene { scene_id, member_id } => CacheObjectRef::ChildScene {
            scene_id: *scene_id,
            member_id: *member_id,
        },
        SceneCacheEntryKind::ChildModel { model_id, member_id } => CacheObjectRef::ChildModel {
            model_id: *model_id,
            member_id: *member_id,
        },
    }
}
