//! Headless Scene-owned cache building plus legacy non-Scene runtime warming.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use openusd::sdf;
use openusd::usd::{InitialLoadSet, Stage};
use serde::Serialize;
use usd_model::HashDigest;
use usd_project::{SceneId, SceneMemberTarget};
use uuid::Uuid;
use viewport_protocol::RuntimeManifest;

use super::cache::{ProjectCacheIdentity, SceneCacheDescriptorV3, SceneCacheStore};
use super::cache_contract::{
    CachedTransform, CacheObjectRef, PROJECT_CACHE_INDEX_SCHEMA_VERSION, ProjectCacheLookup,
    SCENE_CACHE_INDEX_SCHEMA_VERSION, SCENE_SPATIAL_INDEX_SCHEMA_VERSION, SceneCacheAddress,
    SceneCacheBlobRef, SceneCacheEntry, SceneCacheEntryKind, SceneCacheIndex, SceneCacheOccurrence,
    SceneSpatialEntry, SceneSpatialIndex,
};
use crate::project::blob_store::{
    BlobStore, FilesystemBlobStore, prepare_mesh_payload,
};

struct OwnedPrimExtraction {
    path: String,
    transform: usd_model::TransformSignature,
    bounds: Option<usd_model::Bounds3>,
    bim_enabled: bool,
    semantic_key: Option<String>,
    mesh: Option<bevy::mesh::Mesh>,
}

#[derive(Serialize)]
struct ProjectCacheIndexV2 {
    schema_version: u16,
    generations: Vec<(SceneId, u64)>,
    rows: Vec<(SceneCacheAddress, CacheObjectRef)>,
}

pub(super) fn build_runtime_cache(
    project_root: &Path,
    path: &Path,
    identity: &ProjectCacheIdentity,
) -> Result<RuntimeManifest> {
    super::cache_preparation::build_runtime_cache(project_root, path, identity)
}

pub(crate) fn build_scene_cache_index(
    project_root: &Path,
    scene_id: SceneId,
    generation: u64,
) -> Result<(SceneCacheIndex, SceneSpatialIndex)> {
    let path = crate::project::scene::authoring::scene_path(project_root, scene_id);
    let members = crate::project::scene::authoring::read_scene_members(&path, scene_id)?;
    let member_roots = members
        .iter()
        .map(|member| crate::project::scene::authoring::scene_member_path(member.id))
        .collect::<HashSet<_>>();
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(path.to_string_lossy().as_ref())
        .context("open Scene-owned cache source")?;
    let mut owned_paths = Vec::new();
    collect_owned_prim_paths(&stage, "/SceneRoot", &member_roots, &mut owned_paths)?;

    let config = usd_semantic::SemanticConfig::default();
    let mut extracted = Vec::with_capacity(owned_paths.len());
    for prim_path in owned_paths {
        let sdf_path = sdf::path(&prim_path)?;
        let transform = usd_semantic::extract_transform(&stage, &sdf_path, &config)?;
        let geometry = usd_semantic::extract_geometry(&stage, &sdf_path, &config)?;
        let (semantic, _) = usd_semantic::extract_metadata(&stage, &sdf_path, &config)?;
        let semantic_key = usd_semantic::resolve_identity(&stage, &sdf_path, &config.identity)
            .ok()
            .map(|(key, _)| key.as_str().to_owned());
        let mesh = if geometry.is_some() {
            usd_bevy::read::geom::read_mesh(&stage, &sdf_path)?
                .map(|read| usd_bevy::mesh::mesh_from_usd(&read))
        } else {
            None
        };
        extracted.push(OwnedPrimExtraction {
            path: prim_path,
            transform,
            bounds: geometry.map(|geometry| geometry.local_bounds),
            bim_enabled: semantic.is_bim_entity(),
            semantic_key,
            mesh,
        });
    }
    drop(stage);

    let store = SceneCacheStore::new(project_root);
    let object_store = store.object_store(scene_id)?;
    let payloads = persist_owned_meshes_parallel(&object_store, &mut extracted)?;
    let mut entries = member_entries(scene_id, members);
    for owned in extracted {
        let payload = payloads.get(&owned.path).cloned();
        let content_hash = payload
            .as_ref()
            .map(|blob| HashDigest::from_hex(&blob.blob_id.0))
            .transpose()
            .context("decode Scene-owned geometry content hash")?;
        entries.push(SceneCacheEntry {
            address: SceneCacheAddress {
                scene_id,
                occurrence: SceneCacheOccurrence::PrimPath(owned.path.clone()),
            },
            parent: None,
            transform: CachedTransform::Prim(owned.transform),
            bounds: owned.bounds,
            cacheable: payload.is_some(),
            bim_enabled: owned.bim_enabled,
            geometry: payload,
            material: None,
            animation: None,
            semantic_key: owned.semantic_key,
            kind: SceneCacheEntryKind::OwnedPrim {
                prim_path: owned.path.clone(),
            },
            content_hash,
        });
    }
    entries.sort_by(|left, right| left.address.cmp(&right.address));
    assign_parent_indexes(&mut entries)?;

    let index = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation,
        entries,
    };
    index.validate(scene_id, generation)?;
    let spatial = build_spatial_index(&index);
    spatial.validate(scene_id, generation)?;
    Ok((index, spatial))
}

pub(crate) fn build_and_publish_scene_cache_generation(
    project_root: &Path,
    descriptor: &SceneCacheDescriptorV3,
) -> Result<SceneCacheIndex> {
    Ok(build_and_publish_scene_cache_generation_inner(project_root, descriptor, false)?
        .expect("unmanaged Scene cache generation must publish"))
}

pub(crate) fn build_and_publish_managed_scene_cache_generation(
    project_root: &Path,
    descriptor: &SceneCacheDescriptorV3,
) -> Result<Option<SceneCacheIndex>> {
    build_and_publish_scene_cache_generation_inner(project_root, descriptor, true)
}

fn build_and_publish_scene_cache_generation_inner(
    project_root: &Path,
    descriptor: &SceneCacheDescriptorV3,
    managed: bool,
) -> Result<Option<SceneCacheIndex>> {
    let (index, spatial) = build_scene_cache_index(project_root, descriptor.scene_id, descriptor.generation)?;
    let payload_bytes = index
        .entries
        .iter()
        .filter_map(|entry| entry.geometry.as_ref())
        .map(|blob| blob.byte_size)
        .sum();
    let expected_descriptor = descriptor.clone();
    let mut descriptor = expected_descriptor.clone();
    descriptor.prim_count = index.entries.len() as u64;
    descriptor.cacheable_count = index.entries.iter().filter(|entry| entry.cacheable).count() as u64;
    descriptor.estimated_cpu_bytes = payload_bytes;
    descriptor.estimated_gpu_bytes = payload_bytes;
    if let Some(expected) = descriptor.source_content_hash {
        let target = super::cache::ProjectCacheTarget::Scene { id: descriptor.scene_id.to_string() };
        ensure!(
            super::cache::target_content_hash(project_root, &target)? == expected,
            "Scene source changed during cache build"
        );
    }
    let store = SceneCacheStore::new(project_root);
    if managed {
        if store
            .publish_generation_if_current(&expected_descriptor, &descriptor, &index, &spatial)?
            .is_none()
        {
            return Ok(None);
        }
    } else {
        store.publish_generation(&descriptor, &index, &spatial)?;
    }
    publish_current_project_cache_lookup(project_root)?;
    Ok(Some(index))
}

pub(crate) fn publish_project_cache_lookup(
    project_root: &Path,
    lookup: &ProjectCacheLookup,
) -> Result<Vec<u8>> {
    let payload = ProjectCacheIndexV2 {
        schema_version: PROJECT_CACHE_INDEX_SCHEMA_VERSION,
        generations: lookup.persistent_generations(),
        rows: lookup.persistent_rows(),
    };
    let bytes = serde_json::to_vec(&payload).context("encode Project cache index")?;
    let layout = crate::project::storage::ProjectStorageLayout::new(project_root);
    let path = layout.project_cache_index_path();
    if fs::read(&path).is_ok_and(|current| current == bytes) {
        return Ok(bytes);
    }
    let parent = path.parent().context("Project cache index has no parent directory")?;
    fs::create_dir_all(parent).context("create Project cache directory")?;
    let temporary = parent.join(format!(".project-index.{}.tmp", Uuid::new_v4()));
    crate::project::catalog::manifest_store::write_bytes_atomic(&temporary, &path, &bytes)
        .context("publish Project cache index")?;
    Ok(bytes)
}

fn collect_owned_prim_paths(
    stage: &Stage,
    parent_path: &str,
    member_roots: &HashSet<String>,
    out: &mut Vec<String>,
) -> Result<()> {
    let parent = stage.prim(sdf::path(parent_path)?);
    let mut children = parent.children()?;
    children.sort_unstable_by(|left, right| left.path().as_str().cmp(right.path().as_str()));
    for child in children {
        let path = child.path().as_str().to_owned();
        if member_roots.contains(&path) {
            continue;
        }
        out.push(path.clone());
        collect_owned_prim_paths(stage, &path, member_roots, out)?;
    }
    Ok(())
}

fn member_entries(
    scene_id: SceneId,
    members: Vec<usd_project::SceneMember>,
) -> Vec<SceneCacheEntry> {
    members
        .into_iter()
        .map(|member| {
            let kind = match member.target {
                SceneMemberTarget::Scene(child) => SceneCacheEntryKind::ChildScene {
                    scene_id: child,
                    member_id: member.id,
                },
                SceneMemberTarget::Model(model) => SceneCacheEntryKind::ChildModel {
                    model_id: model,
                    member_id: member.id,
                },
            };
            SceneCacheEntry {
                address: SceneCacheAddress {
                    scene_id,
                    occurrence: SceneCacheOccurrence::Member(member.id),
                },
                parent: None,
                transform: CachedTransform::Placement(member.transform),
                bounds: None,
                cacheable: false,
                bim_enabled: false,
                geometry: None,
                material: None,
                animation: None,
                semantic_key: None,
                kind,
                content_hash: None,
            }
        })
        .collect()
}

pub(crate) fn assign_parent_indexes(entries: &mut [SceneCacheEntry]) -> Result<()> {
    let path_indexes = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| match &entry.kind {
            SceneCacheEntryKind::OwnedPrim { prim_path } => Some((prim_path.clone(), index as u32)),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    for entry in entries {
        let SceneCacheEntryKind::OwnedPrim { prim_path } = &entry.kind else {
            continue;
        };
        let parent_path = prim_path.rsplit_once('/').map(|(parent, _)| parent).unwrap_or_default();
        entry.parent = path_indexes.get(parent_path).copied();
    }
    Ok(())
}

pub(crate) fn build_spatial_index(index: &SceneCacheIndex) -> SceneSpatialIndex {
    SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id: index.scene_id,
        generation: index.generation,
        entries: index
            .entries
            .iter()
            .filter_map(|entry| entry.bounds.map(|bounds| SceneSpatialEntry {
                address: entry.address.clone(),
                bounds,
            }))
            .collect(),
    }
}

fn persist_owned_meshes_parallel(
    store: &FilesystemBlobStore,
    extracted: &mut [OwnedPrimExtraction],
) -> Result<HashMap<String, SceneCacheBlobRef>> {
    let meshes = extracted
        .iter_mut()
        .filter_map(|owned| owned.mesh.take().map(|mesh| (owned.path.clone(), mesh)))
        .collect::<Vec<_>>();
    if meshes.is_empty() {
        return Ok(HashMap::new());
    }
    let worker_count = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(4)
        .min(meshes.len());
    let chunk_size = meshes.len().div_ceil(worker_count);
    let chunks = std::thread::scope(|scope| -> Result<Vec<Vec<(String, SceneCacheBlobRef)>>> {
        let mut handles = Vec::new();
        for chunk in meshes.chunks(chunk_size) {
            let store = store.clone();
            handles.push(scope.spawn(move || -> Result<Vec<(String, SceneCacheBlobRef)>> {
                let mut persisted = Vec::with_capacity(chunk.len());
                for (path, mesh) in chunk {
                    let prepared = prepare_mesh_payload(mesh)?;
                    let stored = store.put(&prepared.bytes)?;
                    ensure!(stored == prepared.blob_id, "Scene geometry digest mismatch");
                    persisted.push((
                        path.clone(),
                        SceneCacheBlobRef {
                            blob_id: stored,
                            byte_size: prepared.bytes.len() as u64,
                        },
                    ));
                }
                Ok(persisted)
            }));
        }
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("Scene cache worker panicked"))
                    .and_then(|result| result)
            })
            .collect()
    })?;
    Ok(chunks.into_iter().flatten().collect())
}

pub(crate) fn publish_current_project_cache_lookup(project_root: &Path) -> Result<Vec<u8>> {
    let manifest = crate::project::catalog::manifest_store::ManifestStore::read_validated(project_root)?;
    let store = SceneCacheStore::new(project_root);
    let mut indexes = Vec::new();
    for scene in manifest.scenes() {
        if let Some(index) = store.load_index(scene.id)? {
            indexes.push(index);
        }
    }
    let lookup = ProjectCacheLookup::from_scene_indexes(&indexes)?;
    publish_project_cache_lookup(project_root, &lookup)
}
