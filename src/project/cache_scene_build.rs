//! Scene-owned cache index construction and payload extraction.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result, ensure};
use bevy::asset::Assets;
use bevy::app::App;
use bevy::image::Image;
use bevy::mesh::Mesh;
use bevy::pbr::StandardMaterial;
use openusd::sdf;
use openusd::usd::{InitialLoadSet, Stage};
use usd_model::HashDigest;
use usd_project::{SceneId, SceneMemberTarget};

use super::blob_store::{BlobStore, FilesystemBlobStore, prepare_mesh_payload};
use super::cache::SceneCacheStore;
use super::cache_contract::{
    CachedTransform, SCENE_CACHE_INDEX_SCHEMA_VERSION, SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
    SceneCacheBlobRef, SceneCacheEntry, SceneCacheEntryKind, SceneCacheIndex,
    SceneSpatialEntry, SceneSpatialIndex,
};
use super::cache_scene_payload::prepare_animation_payload;
use super::runtime_payload::{PreparedRuntimePayloads, prepare_runtime_payloads_for_paths};

pub(crate) struct OwnedPrimExtraction {
    pub(crate) path: String,
    pub(crate) transform: usd_model::TransformSignature,
    pub(crate) bounds: Option<usd_model::Bounds3>,
    pub(crate) bim_enabled: bool,
    pub(crate) semantic_key: Option<String>,
    pub(crate) mesh: Option<Mesh>,
}

#[derive(Default)]
struct ScenePayloadRefs {
    material_by_path: HashMap<String, SceneCacheBlobRef>,
    animation_by_path: HashMap<String, SceneCacheBlobRef>,
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
    for prim_path in &owned_paths {
        let sdf_path = sdf::path(prim_path)?;
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
            path: prim_path.clone(),
            transform,
            bounds: geometry.map(|geometry| geometry.local_bounds),
            bim_enabled: semantic.is_bim_entity(),
            semantic_key,
            mesh,
        });
    }

    let payload_paths = extracted
        .iter()
        .filter(|owned| owned.mesh.is_some())
        .map(|owned| owned.path.clone())
        .collect::<Vec<_>>();
    let prepared = prepare_scene_payloads(stage, &path, &payload_paths)?;
    let store = SceneCacheStore::new(project_root);
    let object_store = store.object_store(scene_id)?;
    let geometry_refs = persist_owned_meshes_parallel(&object_store, &mut extracted)?;
    let payload_refs = persist_scene_payloads(&object_store, &prepared)?;

    let mut entries = member_entries(scene_id, members);
    for owned in extracted {
        let geometry = geometry_refs.get(&owned.path).cloned();
        let content_hash = geometry
            .as_ref()
            .map(|blob| HashDigest::from_hex(&blob.blob_id.0))
            .transpose()
            .context("decode Scene-owned geometry content hash")?;
        entries.push(SceneCacheEntry {
            address: super::cache_contract::SceneCacheAddress {
                scene_id,
                occurrence: super::cache_contract::SceneCacheOccurrence::PrimPath(
                    owned.path.clone(),
                ),
            },
            parent: None,
            transform: CachedTransform::Prim(owned.transform),
            bounds: owned.bounds,
            cacheable: geometry.is_some(),
            bim_enabled: owned.bim_enabled,
            geometry,
            material: payload_refs.material_by_path.get(&owned.path).cloned(),
            animation: payload_refs.animation_by_path.get(&owned.path).cloned(),
            semantic_key: owned.semantic_key,
            kind: SceneCacheEntryKind::OwnedPrim {
                prim_path: owned.path,
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

fn prepare_scene_payloads(
    stage: Stage,
    stage_path: &Path,
    owned_paths: &[String],
) -> Result<(PreparedRuntimePayloads, HashMap<String, Vec<u8>>)> {
    let mut app = App::new();
    app.add_plugins(usd_bevy::UsdPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>();
    let archives = usd_bevy::route::material::archive_paths_for_stage(&stage, stage_path)
        .unwrap_or_else(|error| {
            bevy::log::warn!(
                "could not derive Scene USDZ packages for {}; material payloads use source fallback: {error:#}",
                stage_path.display()
            );
            Vec::new()
        });
    app.world_mut()
        .resource_mut::<usd_bevy::route::material::UsdTextureCache>()
        .replace_active_archives(archives);
    let live = usd_bevy::LiveStage::new(stage);
    let mut entities = usd_bevy::PrimEntities::default();
    let requested = owned_paths.iter().map(String::as_str).collect::<Vec<_>>();
    if !requested.is_empty() {
        usd_bevy::project_paths(app.world_mut(), &live, &mut entities, &requested)
            .context("project Scene-owned material payloads")?;
    }
    let materials = prepare_runtime_payloads_for_paths(app.world_mut(), owned_paths);
    let animations = owned_paths
        .iter()
        .filter_map(|path| {
            prepare_animation_payload(&live.stage, path)
                .transpose()
                .map(|result| result.map(|bytes| (path.clone(), bytes)))
        })
        .collect::<Result<HashMap<_, _>>>()?;
    Ok((materials, animations))
}

fn persist_scene_payloads(
    store: &FilesystemBlobStore,
    (materials, animations): &(PreparedRuntimePayloads, HashMap<String, Vec<u8>>),
) -> Result<ScenePayloadRefs> {
    let mut refs = ScenePayloadRefs::default();
    let material_ids = materials
        .materials
        .iter()
        .chain(materials.textures.iter())
        .map(|payload| (&payload.blob_id, &payload.bytes));
    for (blob_id, bytes) in material_ids {
        let stored = store.put(bytes)?;
        ensure!(stored == *blob_id, "Scene material/texture digest mismatch");
    }
    for (path, bytes) in animations {
        let blob_id = store.put(bytes)?;
        refs.animation_by_path.insert(
            path.clone(),
            SceneCacheBlobRef {
                blob_id,
                byte_size: bytes.len() as u64,
            },
        );
    }
    for (path, blob_id) in &materials.material_by_entity {
        let blob_id = usd_model::BlobId(blob_id.clone());
        let bytes = materials
            .materials
            .iter()
            .find(|payload| payload.blob_id == blob_id)
            .map(|payload| payload.bytes.len() as u64)
            .context("material payload missing from prepared Scene batch")?;
        refs.material_by_path.insert(
            path.clone(),
            SceneCacheBlobRef {
                blob_id,
                byte_size: bytes,
            },
        );
    }
    Ok(refs)
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
                address: super::cache_contract::SceneCacheAddress {
                    scene_id,
                    occurrence: super::cache_contract::SceneCacheOccurrence::Member(member.id),
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

pub(crate) fn persist_owned_meshes_parallel(
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
