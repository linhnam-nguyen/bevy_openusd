//! Persistent render-blob references used by historical diff ghosts.
//!
//! Semantic extraction stays renderer-neutral. This small application-side
//! adapter runs after the current Bevy projection exists and attaches the
//! content address of a projected mesh to the corresponding semantic entity.
//! The scene overlay can hydrate that blob without opening another `LiveStage`.

use std::collections::HashMap;

use bevy::asset::Assets;
use bevy::mesh::{Mesh, Mesh3d};
use bevy::prelude::{Resource, World};
use usd_bevy::UsdPrimRef;
use usd_model::SemanticSnapshot;

use super::blob_store::{
    FilesystemBlobStore, OBJECTS_DIRECTORY, PreparedMeshBlob, prepare_mesh_payload, put_mesh,
};
use super::recovery::RecoverySettings;

/// Runtime counters for historical geometry capture.
///
/// The resource is intentionally a small diagnostics surface. The frontend
/// must receive a future read-model projection rather than inspecting this
/// Bevy resource directly.
#[derive(Debug, Default, Resource, Clone, Copy, Eq, PartialEq)]
pub(crate) struct HistoricalGeometryCache {
    pub(crate) snapshots_seen: u64,
    pub(crate) blob_references_attached: u64,
    pub(crate) capture_failures: u64,
    pub(crate) ghost_mesh_hydrations: u64,
    pub(crate) ghost_load_failures: u64,
    pub(crate) mesh_handles_scanned: u64,
    pub(crate) semantic_entities_scanned: u64,
}

/// Attach persistent render-blob identities to the semantic snapshot's mesh
/// entities. Existing references are retained because they belong to the
/// extracted historical/current snapshot already being synchronized.
pub(crate) fn attach_render_blobs(world: &mut World, snapshot: &mut SemanticSnapshot) {
    let Some(project_root) = world
        .get_resource::<RecoverySettings>()
        .map(|settings| settings.project_root.clone())
    else {
        return;
    };
    let Some(mesh_handles) = collect_mesh_handles(world) else {
        return;
    };
    let Some(meshes) = world.get_resource::<Assets<Mesh>>() else {
        return;
    };

    let store = match FilesystemBlobStore::new(project_root.join(OBJECTS_DIRECTORY)) {
        Ok(store) => store,
        Err(error) => {
            bevy::log::error!("[ghost-cache] cannot create mesh blob store: {error:#}");
            return;
        }
    };

    let handle_count = mesh_handles.len() as u64;
    let entity_count = snapshot.entities.len() as u64;
    let mut captured = HashMap::new();
    let mut failures = 0;
    for (path, handle) in mesh_handles {
        let Some(mesh) = meshes.get(&handle) else {
            continue;
        };
        match put_mesh(&store, mesh) {
            Ok(blob_id) => {
                captured.entry(path).or_insert(blob_id);
            }
            Err(error) => {
                failures += 1;
                bevy::log::debug!(
                    "[ghost-cache] mesh at {} is not blob-serializable: {error:#}",
                    handle.id()
                );
            }
        }
    }
    let mut attached = 0;
    for entity in snapshot.entities.values_mut() {
        let Some(geometry) = entity.geometry.as_mut() else {
            continue;
        };
        if geometry.render_blob.is_some() {
            continue;
        }
        if let Some(blob_id) = captured.get(&entity.prim_path) {
            geometry.render_blob = Some(blob_id.clone());
            attached += 1;
        }
    }

    if let Some(mut cache) = world.get_resource_mut::<HistoricalGeometryCache>() {
        cache.snapshots_seen += 1;
        cache.blob_references_attached += attached;
        cache.capture_failures += failures;
        cache.mesh_handles_scanned += handle_count;
        cache.semantic_entities_scanned += entity_count;
    }
}

/// Attach persistent render-blob identities to a specific slice of semantic entities (e.g. upserts in a Delta).
///
/// This avoids scanning all Bevy mesh handles and the entire semantic snapshot when only
/// a subtree or specific set of prims were updated.
pub(crate) fn attach_render_blobs_for_entities(
    world: &mut World,
    entities: &mut [usd_model::EntitySnapshot],
) {
    if entities.is_empty() {
        return;
    }
    let Some(project_root) = world
        .get_resource::<RecoverySettings>()
        .map(|settings| settings.project_root.clone())
    else {
        return;
    };

    let Some(map) = world.get_resource::<usd_bevy::PrimEntities>() else {
        bevy::log::warn!(
            target: "ghost_cache",
            resync_fallback_reason = "missing_prim_entities_index",
            "[ghost-cache] attach_render_blobs_for_entities called without PrimEntities resource"
        );
        return;
    };

    let store = match FilesystemBlobStore::new(project_root.join(OBJECTS_DIRECTORY)) {
        Ok(store) => store,
        Err(error) => {
            bevy::log::error!("[ghost-cache] cannot create mesh blob store: {error:#}");
            return;
        }
    };

    let entity_count = entities.len() as u64;

    // Collect mesh handle ONLY for the affected entities using O(1) PrimEntities index lookup
    let mut mesh_handles = HashMap::new();
    for entity in entities.iter() {
        if let Some(geometry) = entity.geometry.as_ref()
            && geometry.render_blob.is_none()
            && let Some(bevy_entity) = map.entity(&entity.prim_path)
            && let Some(mesh_3d) = world.get::<Mesh3d>(bevy_entity)
        {
            mesh_handles.insert(entity.prim_path.clone(), mesh_3d.0.clone());
        }
    }

    let Some(meshes) = world.get_resource::<Assets<Mesh>>() else {
        return;
    };

    let handle_count = mesh_handles.len() as u64;
    let mut captured = HashMap::new();
    let mut failures = 0;
    for (path, handle) in mesh_handles {
        let Some(mesh) = meshes.get(&handle) else {
            continue;
        };
        match put_mesh(&store, mesh) {
            Ok(blob_id) => {
                captured.entry(path).or_insert(blob_id);
            }
            Err(error) => {
                failures += 1;
                bevy::log::debug!(
                    "[ghost-cache] mesh at {} is not blob-serializable: {error:#}",
                    handle.id()
                );
            }
        }
    }

    let mut attached = 0;
    for entity in entities.iter_mut() {
        let Some(geometry) = entity.geometry.as_mut() else {
            continue;
        };
        if geometry.render_blob.is_some() {
            continue;
        }
        if let Some(blob_id) = captured.get(&entity.prim_path) {
            geometry.render_blob = Some(blob_id.clone());
            attached += 1;
        }
    }

    if let Some(mut cache) = world.get_resource_mut::<HistoricalGeometryCache>() {
        cache.snapshots_seen += 1;
        cache.blob_references_attached += attached;
        cache.capture_failures += failures;
        cache.mesh_handles_scanned += handle_count;
        cache.semantic_entities_scanned += entity_count;
    }
}

/// Prepare mesh payloads and attach their content identities without any
/// filesystem access. The returned immutable descriptors are safe to hand to
/// the bounded runtime-delivery worker.
pub(crate) fn prepare_render_blobs(
    world: &mut World,
    snapshot: &mut SemanticSnapshot,
) -> Vec<PreparedMeshBlob> {
    let Some(mesh_handles) = collect_mesh_handles(world) else {
        return Vec::new();
    };
    let Some(meshes) = world.get_resource::<Assets<Mesh>>() else {
        return Vec::new();
    };

    let handle_count = mesh_handles.len() as u64;
    let entity_count = snapshot.entities.len() as u64;
    let mut captured = HashMap::new();
    let mut prepared = HashMap::new();
    let mut failures = 0;
    for (path, handle) in mesh_handles {
        let Some(mesh) = meshes.get(&handle) else {
            continue;
        };
        match prepare_mesh_payload(mesh) {
            Ok(payload) => {
                captured
                    .entry(path.clone())
                    .or_insert_with(|| payload.blob_id.clone());
                prepared.entry(path).or_insert(payload);
            }
            Err(error) => {
                failures += 1;
                bevy::log::debug!(
                    "[ghost-cache] mesh at {} is not blob-serializable: {error:#}",
                    handle.id()
                );
            }
        }
    }
    let mut attached = 0;
    for entity in snapshot.entities.values_mut() {
        let Some(geometry) = entity.geometry.as_mut() else {
            continue;
        };
        if geometry.render_blob.is_some() {
            continue;
        }
        if let Some(blob_id) = captured.get(&entity.prim_path) {
            geometry.render_blob = Some(blob_id.clone());
            attached += 1;
        }
    }

    if let Some(mut cache) = world.get_resource_mut::<HistoricalGeometryCache>() {
        cache.snapshots_seen += 1;
        cache.blob_references_attached += attached;
        cache.capture_failures += failures;
        cache.mesh_handles_scanned += handle_count;
        cache.semantic_entities_scanned += entity_count;
    }
    prepared.into_values().collect()
}

/// Prepare only the mesh payloads for affected semantic upserts. This keeps
/// the owner-thread work sparse while leaving all persistence to the worker.
pub(crate) fn prepare_render_blobs_for_entities(
    world: &mut World,
    entities: &mut [usd_model::EntitySnapshot],
) -> Vec<PreparedMeshBlob> {
    if entities.is_empty() {
        return Vec::new();
    }
    let Some(map) = world.get_resource::<usd_bevy::PrimEntities>() else {
        return Vec::new();
    };

    let mut mesh_handles = HashMap::new();
    for entity in entities.iter() {
        if let Some(geometry) = entity.geometry.as_ref()
            && geometry.render_blob.is_none()
            && let Some(bevy_entity) = map.entity(&entity.prim_path)
            && let Some(mesh_3d) = world.get::<Mesh3d>(bevy_entity)
        {
            mesh_handles.insert(entity.prim_path.clone(), mesh_3d.0.clone());
        }
    }
    let Some(meshes) = world.get_resource::<Assets<Mesh>>() else {
        return Vec::new();
    };

    let entity_count = entities.len() as u64;
    let handle_count = mesh_handles.len() as u64;
    let mut captured = HashMap::new();
    let mut prepared = HashMap::new();
    let mut failures = 0;
    for (path, handle) in mesh_handles {
        let Some(mesh) = meshes.get(&handle) else {
            continue;
        };
        match prepare_mesh_payload(mesh) {
            Ok(payload) => {
                captured
                    .entry(path.clone())
                    .or_insert_with(|| payload.blob_id.clone());
                prepared.entry(path).or_insert(payload);
            }
            Err(error) => {
                failures += 1;
                bevy::log::debug!(
                    "[ghost-cache] mesh at {} is not blob-serializable: {error:#}",
                    handle.id()
                );
            }
        }
    }
    let mut attached = 0;
    for entity in entities.iter_mut() {
        let Some(geometry) = entity.geometry.as_mut() else {
            continue;
        };
        if geometry.render_blob.is_some() {
            continue;
        }
        if let Some(blob_id) = captured.get(&entity.prim_path) {
            geometry.render_blob = Some(blob_id.clone());
            attached += 1;
        }
    }

    if let Some(mut cache) = world.get_resource_mut::<HistoricalGeometryCache>() {
        cache.snapshots_seen += 1;
        cache.blob_references_attached += attached;
        cache.capture_failures += failures;
        cache.mesh_handles_scanned += handle_count;
        cache.semantic_entities_scanned += entity_count;
    }
    prepared.into_values().collect()
}

fn collect_mesh_handles(world: &mut World) -> Option<HashMap<String, bevy::asset::Handle<Mesh>>> {
    let mut query = world.query::<(&UsdPrimRef, &Mesh3d)>();
    let handles = query
        .iter(world)
        .map(|(prim, mesh)| (prim.path.clone(), mesh.0.clone()))
        .collect();
    Some(handles)
}

#[cfg(test)]
mod tests;
