//! Conservative mark-and-sweep garbage collection for derived Project cache
//! artifacts.
//!
//! Tracks references from current pointers, retained historical revisions,
//! Project descriptors, generated LOD catalogues, and active reader/builder
//! pins. Aborts conservatively if any reachability root is unreadable or
//! uncertain, and refuses to delete symlinks or any path outside the
//! OS-canonicalized `.usdhub/cache/` root.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::{Context, Result, ensure};
use usd_model::BlobId;
use usd_project::SceneId;

use super::blobs::BlobStore;
use super::contract::SceneCacheState;
use super::descriptor::ProjectCacheDescriptor;
use super::retention::{ImmutableRevisionStore, load_active_pins};
use super::scene::store::SceneCacheStore;
use super::sync::{TombstoneStore, is_path_alias, qualify_cache_entry, qualified_cache_root, with_cache_lock};
use crate::project::storage::ProjectStorageLayout;

/// Retention policy governing how many historical revisions per target are
/// kept during garbage collection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GcPolicy {
    pub(crate) max_retained_revisions_per_target: usize,
    pub(crate) collect_orphaned_temps: bool,
}

impl Default for GcPolicy {
    fn default() -> Self {
        Self {
            max_retained_revisions_per_target: 4,
            collect_orphaned_temps: true,
        }
    }
}

/// Summary of one garbage-collection pass.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct GcReport {
    pub(crate) retained_revisions: usize,
    pub(crate) removed_revisions: usize,
    pub(crate) retained_blobs: usize,
    pub(crate) removed_blobs: usize,
    pub(crate) removed_temps: usize,
    pub(crate) aborted_reason: Option<String>,
}

/// Run conservative mark-and-sweep garbage collection over `.usdhub/cache/`.
///
/// Never modifies or deletes canonical Project files (`project.json`,
/// `scenes/`, `models/`, `imports/`, `.git/`). If any reachability root cannot
/// be verified, aborts safely with `aborted_reason = Some(...)` and deletes
/// zero objects.
pub(crate) fn collect_garbage(project_root: &Path, policy: GcPolicy) -> Result<GcReport> {
    let layout = ProjectStorageLayout::new(project_root);
    let Some(cache_dir) = qualified_cache_root(project_root, false)? else {
        return Ok(GcReport::default());
    };
    with_cache_lock(project_root, "gc", || {
        collect_garbage_locked(project_root, &layout, &cache_dir, policy)
    })
}

fn collect_garbage_locked(
    project_root: &Path,
    layout: &ProjectStorageLayout,
    cache_dir: &Path,
    policy: GcPolicy,
) -> Result<GcReport> {
    let canonical_cache_root = qualified_cache_root(project_root, false)?
        .context("missing qualified cache root")?;
    ensure!(canonical_cache_root == cache_dir, "cache root changed before GC");

    let mut report = GcReport::default();

    let pins = match load_active_pins(project_root) {
        Ok(pins) => pins,
        Err(error) => {
            report.aborted_reason = Some(format!("uncertain pin reachability: {error:#}"));
            return Ok(report);
        }
    };
    let mut pinned_revisions = BTreeSet::new();
    let mut reachable_blobs = BTreeSet::<BlobId>::new();
    for pin in &pins {
        pinned_revisions.extend(pin.revision_keys.iter().cloned());
        reachable_blobs.extend(pin.blob_ids.iter().cloned());
    }

    let revision_store = ImmutableRevisionStore::new(project_root);
    let all_revisions = match revision_store.list_revisions() {
        Ok(revisions) => revisions,
        Err(error) => {
            report.aborted_reason = Some(format!("uncertain revision reachability: {error:#}"));
            return Ok(report);
        }
    };

    let mut current_revision_keys = BTreeSet::new();
    if let Err(error) = collect_current_scene_roots(
        project_root,
        &layout,
        &mut current_revision_keys,
        &mut reachable_blobs,
    ) {
        report.aborted_reason = Some(format!("uncertain current Scene reachability: {error:#}"));
        return Ok(report);
    }

    if let Err(error) = collect_project_descriptor_roots(&layout, &mut reachable_blobs) {
        report.aborted_reason = Some(format!(
            "uncertain Project descriptor reachability: {error:#}"
        ));
        return Ok(report);
    }

    // Partition immutable revisions into retained vs. eligible-for-sweep per target.
    // Pass 1: always retain active current pointers and pinned revisions first so
    // they take priority over unpinned historical revisions.
    let mut per_target_kept = BTreeMap::<String, usize>::new();
    let mut retained_revision_keys = BTreeSet::new();
    let mut unpinned_candidates = Vec::new();
    for revision in &all_revisions {
        let is_pinned = pinned_revisions.contains(&revision.revision_key)
            || current_revision_keys.contains(&revision.revision_key);
        if is_pinned {
            *per_target_kept
                .entry(revision.identity.target.key())
                .or_insert(0) += 1;
            retained_revision_keys.insert(revision.revision_key.clone());
            reachable_blobs.extend(revision.referenced_blobs.iter().cloned());
        } else {
            unpinned_candidates.push(revision);
        }
    }
    // Pass 2: retain up to `max_retained_revisions_per_target` per target
    // (newest generation first) and mark excess unpinned revisions removable.
    let mut removable_revisions = Vec::new();
    for revision in unpinned_candidates {
        let kept_count = per_target_kept
            .entry(revision.identity.target.key())
            .or_insert(0);
        if *kept_count < policy.max_retained_revisions_per_target {
            *kept_count += 1;
            retained_revision_keys.insert(revision.revision_key.clone());
            reachable_blobs.extend(revision.referenced_blobs.iter().cloned());
        } else {
            removable_revisions.push(revision.revision_key.clone());
        }
    }

    // Pin admission and all shared CAS publication use the same transaction;
    // no new roots can become visible until this sweep completes.
    removable_revisions.retain(|key| !retained_revision_keys.contains(key));

    for revision_key in removable_revisions {
        let dir = revision_store.revision_dir(&revision_key);
        safe_remove_cache_dir(&canonical_cache_root, &dir)?;
        report.removed_revisions += 1;
    }
    report.retained_revisions = retained_revision_keys.len();

    // Sweep only unreferenced blobs in the shared CAS pool.
    let shared_objects = layout.shared_cache_objects_dir();
    sweep_blob_pool(
        &canonical_cache_root,
        &shared_objects,
        &reachable_blobs,
        &mut report,
    )?;
    // Legacy readers do not all own durable leases yet. Their top-level and
    // per-Scene pools are excluded from destructive GC until their migration.
    // Do not recursively sweep arbitrary temporary names used by other owners.
    if policy.collect_orphaned_temps && pins.is_empty() {
        sweep_orphaned_temps(&canonical_cache_root, &cache_dir, &mut report)?;
    }

    Ok(report)
}

fn collect_current_scene_roots(
    project_root: &Path,
    layout: &ProjectStorageLayout,
    current_revision_keys: &mut BTreeSet<String>,
    reachable_blobs: &mut BTreeSet<BlobId>,
) -> Result<()> {
    let scenes_dir = layout.cache_dir().join("scenes");
    if !super::sync::qualify_cache_directory(&scenes_dir, false)? { return Ok(()); }
    let entries = match fs::read_dir(&scenes_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let scene_store = SceneCacheStore::new(project_root);
    let tombstones = TombstoneStore::new(project_root);
    for entry in entries {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            !is_path_alias(&metadata),
            "refusing symlink in Scene cache directory: {}",
            path.display()
        );
        if !metadata.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Ok(scene_id) = SceneId::parse(name) else {
            continue;
        };
        let target = super::contract::ProjectCacheTarget::Scene {
            id: scene_id.to_string(),
        };
        let Some(descriptor) = scene_store.load_descriptor(scene_id)? else {
            continue;
        };
        if descriptor.state == SceneCacheState::Retired
            || tombstones.is_retired_at(&target, descriptor.generation)?
        {
            continue;
        }
        if let Some(rev_digest) = descriptor.revision_identity {
            current_revision_keys.insert(rev_digest.to_hex());
        }
        if matches!(
            descriptor.state,
            SceneCacheState::Ready | SceneCacheState::Partial
        ) {
            let index = scene_store
                .load_index(scene_id)?
                .with_context(|| format!("missing index for active Scene cache {scene_id}"))?;
            let scene_objects = scene_store.object_store(scene_id)?;
            let shared_objects = scene_store.shared_object_store()?;
            for entry in &index.entries {
                if let Some(geometry) = &entry.geometry {
                    reachable_blobs.insert(geometry.blob_id.clone());
                }
                if let Some(animation) = &entry.animation {
                    reachable_blobs.insert(animation.blob_id.clone());
                }
                if let Some(material) = &entry.material {
                    reachable_blobs.insert(material.blob_id.clone());
                    let material_bytes = scene_objects
                        .get(&material.blob_id)?
                        .or(shared_objects.get(&material.blob_id)?)
                        .with_context(|| {
                            format!("missing material blob {} for Scene {scene_id}", material.blob_id.0)
                        })?;
                    let parsed: crate::document::runtime::payload::RuntimeMaterialBlob =
                        serde_json::from_slice(&material_bytes)?;
                    parsed.validate()?;
                    for texture_id in [
                        parsed.textures.base_color.as_deref(),
                        parsed.textures.normal.as_deref(),
                        parsed.textures.metallic_roughness.as_deref(),
                        parsed.textures.emissive.as_deref(),
                        parsed.textures.occlusion.as_deref(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        reachable_blobs.insert(BlobId(texture_id.to_owned()));
                    }
                }
            }
            if let Some(lods) =
                super::derived_lod::read_scene_lods_checked(project_root, scene_id, descriptor.generation)?
            {
                for (path, manifest) in &lods.candidates {
                    ensure!(index.entries.iter().any(|entry| {
                        matches!(&entry.kind, super::contract::SceneCacheEntryKind::OwnedPrim { prim_path } if prim_path == path)
                            && entry.geometry.as_ref().is_some_and(|geometry| geometry.blob_id == manifest.source_blob)
                    }), "generated LOD catalogue does not match the current Scene index");
                    // Generated LODs are written to the legacy Scene object pool,
                    // not the swept shared pool. Mark their digests conservatively
                    // nevertheless, and refuse uncertain references.
                    for id in [&manifest.source_blob, &manifest.visual_blob] {
                        ensure!(scene_objects.contains(id)?, "missing generated LOD resource {}", id.0);
                        reachable_blobs.insert(id.clone());
                    }
                }
            }
        }
    }
    Ok(())
}

fn collect_project_descriptor_roots(
    layout: &ProjectStorageLayout,
    reachable_blobs: &mut BTreeSet<BlobId>,
) -> Result<()> {
    let descriptors_dir = layout.cache_dir().join("descriptors");
    if !super::sync::qualify_cache_directory(&descriptors_dir, false)? { return Ok(()); }
    let entries = match fs::read_dir(&descriptors_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            !is_path_alias(&metadata),
            "refusing symlink in descriptors directory: {}",
            path.display()
        );
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path)?;
        let descriptor: ProjectCacheDescriptor = serde_json::from_slice(&bytes)?;
        descriptor.validate()?;
        if let Some(runtime) = &descriptor.runtime {
            reachable_blobs.insert(BlobId(runtime.hierarchy.blob_id.clone()));
            for reference in runtime
                .meshes
                .iter()
                .chain(runtime.materials.iter())
                .chain(runtime.textures.iter())
            {
                reachable_blobs.insert(BlobId(reference.blob_id.clone()));
            }
        }
    }
    Ok(())
}

fn sweep_blob_pool(
    canonical_cache_root: &Path,
    pool_root: &Path,
    reachable_blobs: &BTreeSet<BlobId>,
    report: &mut GcReport,
) -> Result<()> {
    if fs::symlink_metadata(pool_root).is_ok() { qualify_cache_entry(canonical_cache_root, pool_root)?; }
    let entries = match fs::read_dir(pool_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let shard_path = entry?.path();
        let metadata = fs::symlink_metadata(&shard_path)?;
        if is_path_alias(&metadata) || !metadata.is_dir() {
            continue;
        }
        let Some(name) = shard_path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_hex_shard_name(name) {
            sweep_shard_dir(canonical_cache_root, &shard_path, reachable_blobs, report)?;
        }
    }
    Ok(())
}

fn sweep_shard_dir(
    canonical_cache_root: &Path,
    shard_dir: &Path,
    reachable_blobs: &BTreeSet<BlobId>,
    report: &mut GcReport,
) -> Result<()> {
    qualify_cache_entry(canonical_cache_root, shard_dir)?;
    let entries = match fs::read_dir(shard_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if is_path_alias(&metadata) || !metadata.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = file_name.strip_suffix(".blob") else {
            continue;
        };
        if stem.len() != 64 || !stem.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
            continue;
        }
        let blob_id = BlobId(stem.to_owned());
        if reachable_blobs.contains(&blob_id) {
            report.retained_blobs += 1;
        } else {
            safe_remove_cache_file(canonical_cache_root, &path)?;
            report.removed_blobs += 1;
        }
    }
    Ok(())
}

fn sweep_orphaned_temps(
    canonical_cache_root: &Path,
    directory: &Path,
    report: &mut GcReport,
) -> Result<()> {
    let revisions = directory.join("revisions");
    match fs::read_dir(&revisions) {
        Ok(entries) => {
            qualify_cache_entry(canonical_cache_root, &revisions)?;
            for entry in entries {
                let path = entry?.path();
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else { continue; };
                if name.starts_with(".revision.") && name.ends_with(".tmp") {
                    safe_remove_cache_dir(canonical_cache_root, &path)?;
                    report.removed_temps += 1;
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(error).context("read revision staging directory"),
    }
    Ok(())
}

fn is_hex_shard_name(name: &str) -> bool {
    name.len() == 2
        && name
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Delete a regular file only after proving via OS filesystem identity that it
/// is not a symlink and its canonical parent resides strictly inside
/// `canonical_cache_root`.
pub(crate) fn safe_remove_cache_file(canonical_cache_root: &Path, path: &Path) -> Result<()> {
    ensure!(qualify_cache_entry(canonical_cache_root, path)?.is_file(),
        "refusing to delete non-regular cache file {}", path.display());
    fs::remove_file(path).with_context(|| format!("remove cache file {}", path.display()))?;
    Ok(())
}

/// Preflight an owned subtree before deleting anything. Never follow links or
/// junctions. The caller holds the Project cache transaction throughout.
pub(crate) fn safe_remove_cache_dir(canonical_cache_root: &Path, dir: &Path) -> Result<()> {
    ensure!(qualify_cache_entry(canonical_cache_root, dir)?.is_dir(), "invalid cache directory");
    let mut stack = vec![dir.to_path_buf()];
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    while let Some(current) = stack.pop() {
        qualify_cache_entry(canonical_cache_root, &current)?;
        for entry in fs::read_dir(&current)? {
            let child = entry?.path();
            let metadata = qualify_cache_entry(canonical_cache_root, &child)?;
            if metadata.is_dir() { stack.push(child); }
            else if metadata.is_file() { files.push(child); }
            else { anyhow::bail!("unknown cache entry type {}", child.display()); }
        }
        dirs.push(current);
    }
    for path in files { safe_remove_cache_file(canonical_cache_root, &path)?; }
    for path in dirs.into_iter().rev() {
        ensure!(qualify_cache_entry(canonical_cache_root, &path)?.is_dir(), "cache directory changed");
        fs::remove_dir(&path).with_context(|| format!("remove cache directory {}", path.display()))?;
    }
    Ok(())
}
