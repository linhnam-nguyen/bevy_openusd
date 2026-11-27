//! Incremental Scene-owned cache index updates built on top of verified
//! immutable base revisions and shared CAS blobs.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use usd_model::HashDigest;
use usd_project::SceneId;

use super::super::SceneCacheStore;
use super::super::changes::{
    FullRebuildReason, IncrementalUpdatePlan, SceneDeltaHint, classify_scene_changes,
};
use super::super::contract::{
    CachedTransform, SCENE_CACHE_INDEX_SCHEMA_VERSION, SceneCacheEntryKind, SceneCacheIndex,
    SceneSpatialIndex,
};
use super::super::identity::CacheRevisionIdentity;
use super::super::retention::{ImmutableRevisionStore, VerifiedRetainedRevision};
use super::build;

/// Telemetry describing which work was reused vs. rebuilt during a Scene cache build.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct IncrementalBuildStats {
    pub(crate) used_incremental: bool,
    pub(crate) fallback_reason: Option<FullRebuildReason>,
    pub(crate) metadata_prims_updated: usize,
    pub(crate) transform_prims_updated: usize,
    pub(crate) geometry_prims_rebuilt: usize,
    pub(crate) geometry_blobs_reused: usize,
    pub(crate) material_prims_rebuilt: usize,
    pub(crate) material_blobs_reused: usize,
    pub(crate) animation_prims_rebuilt: usize,
    pub(crate) animation_blobs_reused: usize,
}

impl IncrementalBuildStats {
    pub(crate) fn full_fallback(reason: FullRebuildReason, index: &SceneCacheIndex) -> Self {
        let geometry_prims_rebuilt = index
            .entries
            .iter()
            .filter(|entry| entry.geometry.is_some())
            .count();
        let material_prims_rebuilt = index
            .entries
            .iter()
            .filter(|entry| entry.material.is_some())
            .count();
        let animation_prims_rebuilt = index
            .entries
            .iter()
            .filter(|entry| entry.animation.is_some())
            .count();
        Self {
            used_incremental: false,
            fallback_reason: Some(reason),
            geometry_prims_rebuilt,
            material_prims_rebuilt,
            animation_prims_rebuilt,
            ..Self::default()
        }
    }
}

/// Build the next Scene cache index incrementally from a verified immutable
/// `base_revision` and persisted `hint`, falling back conservatively to a clean
/// full build whenever base compatibility, dependency closure, or change
/// classification is uncertain.
pub(crate) fn build_scene_cache_index_incremental_from_snapshot(
    project_root: &Path,
    scene_id: SceneId,
    generation: u64,
    live: &usd_bevy::LiveStage,
    base_revision: &VerifiedRetainedRevision,
    target_revision: &CacheRevisionIdentity,
    hint: &SceneDeltaHint,
) -> Result<(SceneCacheIndex, SceneSpatialIndex, IncrementalBuildStats)> {
    if base_revision.index.scene_id != scene_id
        || !hint.matches_revisions(&base_revision.record.identity, target_revision)
    {
        return full_fallback(
            project_root,
            scene_id,
            generation,
            live,
            FullRebuildReason::BaseRevisionCorruptOrIncompatible,
        );
    }

    let stage = &live.stage;
    let members = crate::project::scene::authoring::read_scene_members_from_stage(stage, scene_id)?;
    let member_roots = members
        .iter()
        .map(|member| crate::project::scene::authoring::scene_member_path(member.id))
        .collect::<HashSet<_>>();

    let mut current_members = build::member_entries(scene_id, members);
    current_members.sort_by(|left, right| left.address.cmp(&right.address));
    let mut base_members = base_revision
        .index
        .entries
        .iter()
        .filter(|entry| !matches!(entry.kind, SceneCacheEntryKind::OwnedPrim { .. }))
        .cloned()
        .collect::<Vec<_>>();
    base_members.sort_by(|left, right| left.address.cmp(&right.address));
    if current_members != base_members {
        return full_fallback(
            project_root,
            scene_id,
            generation,
            live,
            FullRebuildReason::MemberEntriesChanged,
        );
    }

    let work = match classify_scene_changes(hint, &base_revision.index, &member_roots) {
        IncrementalUpdatePlan::FullRebuild { reason } => {
            return full_fallback(project_root, scene_id, generation, live, reason);
        }
        IncrementalUpdatePlan::Incremental(work) => work,
    };

    // Verify sealed resolver bindings before reusing or persisting payloads.
    live.opened_resolver_bindings()?
        .context("Scene extraction has no sealed resolver bindings")?;

    let store = SceneCacheStore::new(project_root);
    let object_store = store.object_store(scene_id)?;
    let revision_store = ImmutableRevisionStore::new(project_root);

    // Ensure every blob referenced by the base revision is available and valid
    // in the per-Scene object store before reusing references.
    if revision_store
        .materialize_blobs_to(&base_revision.record.referenced_blobs, &object_store)
        .is_err()
        || ImmutableRevisionStore::verify_blobs_present(
            &object_store,
            &base_revision.record.referenced_blobs,
        )
        .is_err()
    {
        return full_fallback(
            project_root,
            scene_id,
            generation,
            live,
            FullRebuildReason::BaseRevisionCorruptOrIncompatible,
        );
    }

    let config = usd_semantic::SemanticConfig::default();
    let mut entries = base_revision.index.entries.clone();
    let mut stats = IncrementalBuildStats {
        used_incremental: true,
        ..IncrementalBuildStats::default()
    };

    // Build path -> entry index lookup for sparse in-place updates.
    let mut index_by_path: HashMap<String, usize> = HashMap::new();
    for (idx, entry) in entries.iter().enumerate() {
        if let SceneCacheEntryKind::OwnedPrim { prim_path } = &entry.kind {
            index_by_path.insert(prim_path.clone(), idx);
        }
    }

    // 1. Metadata-only updates (never touch geometry, material, or animation).
    for prim_path in &work.metadata_prims {
        let Some(&idx) = index_by_path.get(prim_path) else {
            return full_fallback(
                project_root,
                scene_id,
                generation,
                live,
                FullRebuildReason::UnknownPrimInfoChange,
            );
        };
        let (bim_enabled, semantic_key) =
            build::extract_owned_prim_metadata(stage, prim_path, &config)?;
        entries[idx].bim_enabled = bim_enabled;
        entries[idx].semantic_key = semantic_key;
        stats.metadata_prims_updated += 1;
    }

    // 2. Transform-only updates (never re-extract or re-encode geometry).
    for prim_path in &work.transform_prims {
        let Some(&idx) = index_by_path.get(prim_path) else {
            return full_fallback(
                project_root,
                scene_id,
                generation,
                live,
                FullRebuildReason::UnknownTransformTarget,
            );
        };
        let transform = build::extract_owned_prim_transform(stage, prim_path, &config)?;
        entries[idx].transform = CachedTransform::Prim(transform);
        stats.transform_prims_updated += 1;
    }

    // 3. Localized geometry updates (re-extract/encode ONLY affected prims).
    let mut geometry_gained_paths = BTreeSet::new();
    if !work.geometry_prims.is_empty() {
        let mut geom_extractions = Vec::with_capacity(work.geometry_prims.len());
        for prim_path in &work.geometry_prims {
            let Some(&idx) = index_by_path.get(prim_path) else {
                return full_fallback(
                    project_root,
                    scene_id,
                    generation,
                    live,
                    FullRebuildReason::UnknownGeometryTarget,
                );
            };
            let had_geometry = entries[idx].geometry.is_some();
            let (bounds, mesh) = build::extract_owned_prim_geometry(stage, prim_path, &config)?;
            let has_mesh = mesh.is_some();
            if has_mesh && !had_geometry {
                geometry_gained_paths.insert(prim_path.clone());
            }
            entries[idx].bounds = bounds;
            geom_extractions.push(build::OwnedPrimExtraction {
                path: prim_path.clone(),
                transform: match &entries[idx].transform {
                    CachedTransform::Prim(transform) => transform.clone(),
                    CachedTransform::Placement(_) => {
                        return full_fallback(
                            project_root,
                            scene_id,
                            generation,
                            live,
                            FullRebuildReason::UnknownGeometryTarget,
                        );
                    }
                },
                bounds,
                bim_enabled: entries[idx].bim_enabled,
                semantic_key: entries[idx].semantic_key.clone(),
                mesh,
            });
        }
        let geometry_refs =
            build::persist_owned_meshes_parallel(&object_store, &mut geom_extractions)?;
        stats.geometry_prims_rebuilt += geometry_refs.len();
        for prim_path in &work.geometry_prims {
            let idx = index_by_path[prim_path];
            let geometry = geometry_refs.get(prim_path).cloned();
            let content_hash = geometry
                .as_ref()
                .map(|blob| HashDigest::from_hex(&blob.blob_id.0))
                .transpose()
                .context("decode incremental Scene-owned geometry content hash")?;
            let has_geometry = geometry.is_some();
            entries[idx].geometry = geometry;
            entries[idx].content_hash = content_hash;
            entries[idx].cacheable = has_geometry;
            if !has_geometry {
                entries[idx].material = None;
                entries[idx].animation = None;
            }
        }
    }

    // 4. Material/texture updates (zero geometry re-extraction).
    let material_targets: Vec<String> = if work.refresh_all_materials {
        entries
            .iter()
            .filter_map(|entry| match &entry.kind {
                SceneCacheEntryKind::OwnedPrim { prim_path }
                    if entry.geometry.is_some() =>
                {
                    Some(prim_path.clone())
                }
                _ => None,
            })
            .collect()
    } else {
        work.material_prims
            .union(&geometry_gained_paths)
            .filter(|path| {
                index_by_path
                    .get(*path)
                    .is_some_and(|&idx| entries[idx].geometry.is_some())
            })
            .cloned()
            .collect()
    };
    if !material_targets.is_empty() {
        let materials = build::prepare_scene_materials(live, &material_targets)?;
        let material_refs = build::persist_material_payloads(&object_store, &materials)?;
        stats.material_prims_rebuilt += material_targets.len();
        for path in &material_targets {
            if let Some(&idx) = index_by_path.get(path) {
                entries[idx].material = material_refs.get(path).cloned();
            }
        }
    }

    // 5. Animation updates (zero static geometry re-extraction).
    let animation_targets: Vec<String> = if work.refresh_all_animations {
        entries
            .iter()
            .filter_map(|entry| match &entry.kind {
                SceneCacheEntryKind::OwnedPrim { prim_path }
                    if entry.geometry.is_some() =>
                {
                    Some(prim_path.clone())
                }
                _ => None,
            })
            .collect()
    } else {
        work.animation_prims
            .union(&geometry_gained_paths)
            .filter(|path| {
                index_by_path
                    .get(*path)
                    .is_some_and(|&idx| entries[idx].geometry.is_some())
            })
            .cloned()
            .collect()
    };
    if !animation_targets.is_empty() {
        let animations = build::prepare_scene_animations(stage, &animation_targets)?;
        let animation_refs = build::persist_animation_payloads(&object_store, &animations)?;
        stats.animation_prims_rebuilt += animation_targets.len();
        for path in &animation_targets {
            if let Some(&idx) = index_by_path.get(path) {
                entries[idx].animation = animation_refs.get(path).cloned();
            }
        }
    }

    // Count reused blobs across unchanged entries.
    let rebuilt_geom: HashSet<&str> = work
        .geometry_prims
        .iter()
        .map(String::as_str)
        .collect();
    let rebuilt_mat: HashSet<&str> = material_targets
        .iter()
        .map(String::as_str)
        .collect();
    let rebuilt_anim: HashSet<&str> = animation_targets
        .iter()
        .map(String::as_str)
        .collect();
    for entry in &entries {
        let SceneCacheEntryKind::OwnedPrim { prim_path } = &entry.kind else {
            continue;
        };
        if entry.geometry.is_some() && !rebuilt_geom.contains(prim_path.as_str()) {
            stats.geometry_blobs_reused += 1;
        }
        if entry.material.is_some() && !rebuilt_mat.contains(prim_path.as_str()) {
            stats.material_blobs_reused += 1;
        }
        if entry.animation.is_some() && !rebuilt_anim.contains(prim_path.as_str()) {
            stats.animation_blobs_reused += 1;
        }
    }

    entries.sort_by(|left, right| left.address.cmp(&right.address));
    build::assign_parent_indexes(&mut entries)?;

    let index = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation,
        entries,
    };
    index.validate(scene_id, generation)?;

    let payload_paths = index
        .entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            SceneCacheEntryKind::OwnedPrim { prim_path } if entry.geometry.is_some() => {
                Some(prim_path.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if let Some(eligible) = build::eligible_lod_paths(stage, &payload_paths)
        && let Err(error) = super::super::derived_lod::prepare_scene_lods(
            project_root,
            scene_id,
            generation,
            &eligible,
            &object_store,
            &index.entries,
        )
    {
        log::warn!("generated LOD preparation kept source geometry: {error:#}");
    }

    let spatial = build::build_spatial_index(&index);
    spatial.validate(scene_id, generation)?;
    Ok((index, spatial, stats))
}

fn full_fallback(
    project_root: &Path,
    scene_id: SceneId,
    generation: u64,
    live: &usd_bevy::LiveStage,
    reason: FullRebuildReason,
) -> Result<(SceneCacheIndex, SceneSpatialIndex, IncrementalBuildStats)> {
    let (index, spatial) =
        build::build_scene_cache_index_from_snapshot(project_root, scene_id, generation, live)?;
    let stats = IncrementalBuildStats::full_fallback(reason, &index);
    Ok((index, spatial, stats))
}

