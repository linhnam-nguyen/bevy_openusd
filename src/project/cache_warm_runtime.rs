//! Headless source projection used to build complete Project runtime caches.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use bevy::{asset::Assets, image::Image, mesh::Mesh, pbr::StandardMaterial, prelude::App};
use openusd::usd::Stage;
use usd_model::SnapshotSource;
use viewport_protocol::{RuntimeManifest, RuntimeProfile};

use super::cache::ProjectCacheIdentity;
use crate::project::blob_store::{
    BlobStore, FilesystemBlobStore, OBJECTS_DIRECTORY, PreparedMeshBlob,
};
use crate::project::ghost_cache::prepare_render_blobs;
use crate::project::runtime_delivery::build_runtime_delivery_with_payloads;
use crate::project::runtime_payload::PreparedRuntimeBlob;

pub(super) fn build_runtime_cache(
    project_root: &Path,
    path: &Path,
    identity: &ProjectCacheIdentity,
) -> Result<RuntimeManifest> {
    let stage_path = path
        .to_str()
        .context("canonical Project stage path must be valid UTF-8")?;
    let stage = Stage::open(stage_path).context("open canonical Project stage for cache warm")?;
    let config = usd_semantic::SemanticConfig::default();
    let mut snapshot = usd_semantic::SemanticExtractor::new(config).extract(
        &stage,
        SnapshotSource::GitCommit {
            oid: identity.target_content_hash.to_string(),
        },
    )?;
    let live = usd_bevy::LiveStage::new(stage);
    let (prepared_meshes, prepared_runtime_payloads) = {
        let mut app = App::new();
        app.add_plugins(usd_bevy::UsdPlugin);
        app.init_resource::<Assets<Mesh>>();
        app.init_resource::<Assets<Image>>();
        app.init_resource::<Assets<StandardMaterial>>();
        let world = app.world_mut();
        let mut prim_entities = usd_bevy::PrimEntities::default();
        usd_bevy::project_stage(world, &live, &mut prim_entities);
        let prepared_meshes = prepare_render_blobs(world, &mut snapshot);
        let prepared_runtime_payloads = super::runtime_payload::prepare_runtime_payloads_for_stage(
            world,
            &live.stage,
            &snapshot,
        );
        (prepared_meshes, prepared_runtime_payloads)
    };
    ensure!(
        prepared_runtime_payloads.complete,
        "canonical Project stage has incomplete runtime material or texture coverage"
    );

    let store = FilesystemBlobStore::new(project_root.join(OBJECTS_DIRECTORY))?;
    let mut persisted = HashSet::new();
    for prepared in &prepared_meshes {
        persist_mesh_blob(&store, prepared, &mut persisted)?;
    }
    for prepared in prepared_runtime_payloads
        .materials
        .iter()
        .chain(prepared_runtime_payloads.textures.iter())
    {
        persist_runtime_blob(&store, prepared, &mut persisted)?;
    }
    let bundle = build_runtime_delivery_with_payloads(
        &store,
        &snapshot,
        RuntimeProfile::NativeMedium,
        &prepared_runtime_payloads,
    )?;
    for (blob_id, bytes) in &bundle.blobs {
        if !persisted.insert(blob_id.clone()) {
            continue;
        }
        let stored = store.put(bytes)?;
        ensure!(
            stored.0 == *blob_id,
            "runtime cache digest mismatch for {blob_id}"
        );
    }
    Ok(bundle.manifest)
}

fn persist_mesh_blob(
    store: &FilesystemBlobStore,
    prepared: &PreparedMeshBlob,
    persisted: &mut HashSet<String>,
) -> Result<()> {
    if !persisted.insert(prepared.blob_id.0.clone()) {
        return Ok(());
    }
    let stored = store.put(&prepared.bytes)?;
    ensure!(
        stored == prepared.blob_id,
        "prepared mesh digest {} was stored as {}",
        prepared.blob_id.0,
        stored.0
    );
    Ok(())
}

fn persist_runtime_blob(
    store: &FilesystemBlobStore,
    prepared: &PreparedRuntimeBlob,
    persisted: &mut HashSet<String>,
) -> Result<()> {
    if !persisted.insert(prepared.blob_id.0.clone()) {
        return Ok(());
    }
    let stored = store.put(&prepared.bytes)?;
    ensure!(
        stored == prepared.blob_id,
        "prepared runtime digest {} was stored as {}",
        prepared.blob_id.0,
        stored.0
    );
    Ok(())
}
