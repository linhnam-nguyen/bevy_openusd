use log::{error, info, warn};
use viewport_protocol::{
    CameraSource, ServerEvent, ServerEventEnvelope, SessionEvent, ViewportReadModel,
};

use super::constants::{
    INITIAL_RUNTIME_BLOB_CHUNK_BYTES, INITIAL_RUNTIME_MANIFEST_CHUNK_REFS,
    INITIAL_SNAPSHOT_CHUNK_PRIMS, MAX_APPLICATION_MESSAGE_BYTES,
    MAX_COMPACT_STAGE_DISPLAY_NAME_CHARS,
};
use super::dispatch::{encoded_size, next_server_envelope};
use super::events::{queue_bounded_event, snapshot_event};
use super::session::ApplicationSessionState;

pub(super) fn queue_runtime_manifest(
    state: &mut ApplicationSessionState,
    request_id: Option<&str>,
    manifest: viewport_protocol::AuthorizedRuntimeManifest,
) {
    let event = ServerEvent::Session(SessionEvent::RuntimeManifest {
        manifest: manifest.clone(),
    });
    let envelope = next_server_envelope(state, request_id, event);
    if encoded_size(&envelope).is_some_and(|size| size <= MAX_APPLICATION_MESSAGE_BYTES) {
        state.pending_server_events.push_back(envelope);
        return;
    }

    state.server_sequence = state.server_sequence.saturating_sub(1);
    let manifest_id = request_id
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("manifest-{}", state.server_sequence));
    let total_references =
        manifest.meshes.len() + manifest.materials.len() + manifest.textures.len();
    let mut chunk_size = INITIAL_RUNTIME_MANIFEST_CHUNK_REFS.max(1);

    loop {
        let chunk_count = total_references.max(1).div_ceil(chunk_size);
        let starting_sequence = state.server_sequence;
        let mesh_offset = 0;
        let material_offset = manifest.meshes.len();
        let texture_offset = material_offset + manifest.materials.len();
        let mut chunks = Vec::with_capacity(chunk_count);

        for chunk_index in 0..chunk_count {
            let start = chunk_index * chunk_size;
            let end = (start + chunk_size).min(total_references);
            let chunk_manifest = viewport_protocol::AuthorizedRuntimeManifest {
                revision: manifest.revision.clone(),
                profile: manifest.profile,
                hierarchy: manifest.hierarchy.clone(),
                meshes: clone_manifest_range(&manifest.meshes, mesh_offset, start, end),
                materials: clone_manifest_range(&manifest.materials, material_offset, start, end),
                textures: clone_manifest_range(&manifest.textures, texture_offset, start, end),
                redacted_blob_count: manifest.redacted_blob_count,
            };
            chunks.push(next_server_envelope(
                state,
                request_id,
                ServerEvent::Session(SessionEvent::RuntimeManifestChunk {
                    manifest_id: manifest_id.clone(),
                    chunk_index: chunk_index as u32,
                    chunk_count: chunk_count as u32,
                    manifest: chunk_manifest,
                }),
            ));
        }

        if chunks.iter().all(|envelope| {
            encoded_size(envelope).is_some_and(|size| size <= MAX_APPLICATION_MESSAGE_BYTES)
        }) {
            state.pending_server_events.extend(chunks);
            return;
        }

        state.server_sequence = starting_sequence;
        if chunk_size == 1 {
            warn!(
                "[viewport-data-channel] dropping runtime manifest {manifest_id} because it exceeds the application message limit"
            );
            return;
        }
        chunk_size = (chunk_size / 2).max(1);
    }
}

pub(super) fn clone_manifest_range<T: Clone>(
    values: &[T],
    offset: usize,
    start: usize,
    end: usize,
) -> Vec<T> {
    let local_start = start.saturating_sub(offset).min(values.len());
    let local_end = end.saturating_sub(offset).min(values.len());
    if local_start >= local_end {
        Vec::new()
    } else {
        values[local_start..local_end].to_vec()
    }
}

pub(super) fn queue_runtime_blob(
    state: &mut ApplicationSessionState,
    request_id: Option<&str>,
    blob_id: String,
    bytes: Vec<u8>,
) {
    let mut chunk_size = INITIAL_RUNTIME_BLOB_CHUNK_BYTES.max(1);
    loop {
        let chunk_count = bytes.len().max(1).div_ceil(chunk_size);
        let starting_sequence = state.server_sequence;
        let mut chunks = Vec::with_capacity(chunk_count);
        if bytes.is_empty() {
            chunks.push(next_server_envelope(
                state,
                request_id,
                ServerEvent::Session(SessionEvent::RuntimeBlobChunk {
                    blob_id: blob_id.clone(),
                    chunk_index: 0,
                    chunk_count: 1,
                    bytes: Vec::new(),
                }),
            ));
        } else {
            for (chunk_index, chunk) in bytes.chunks(chunk_size).enumerate() {
                chunks.push(next_server_envelope(
                    state,
                    request_id,
                    ServerEvent::Session(SessionEvent::RuntimeBlobChunk {
                        blob_id: blob_id.clone(),
                        chunk_index: chunk_index as u32,
                        chunk_count: chunk_count as u32,
                        bytes: chunk.to_vec(),
                    }),
                ));
            }
        }

        if chunks.iter().all(|envelope| {
            encoded_size(envelope).is_some_and(|size| size <= MAX_APPLICATION_MESSAGE_BYTES)
        }) {
            state.pending_server_events.extend(chunks);
            return;
        }

        state.server_sequence = starting_sequence;
        if chunk_size == 1 {
            warn!(
                "[viewport-data-channel] dropping runtime blob {blob_id} because it exceeds the application message limit"
            );
            return;
        }
        chunk_size = (chunk_size / 2).max(1);
    }
}

pub(super) fn queue_snapshot(
    state: &mut ApplicationSessionState,
    request_id: Option<String>,
    mut snapshot: ViewportReadModel,
    session_snapshot: bool,
) {
    let event = snapshot_event(snapshot.clone(), session_snapshot);
    let envelope = next_server_envelope(state, request_id.as_deref(), event);
    if encoded_size(&envelope).is_some_and(|size| size <= MAX_APPLICATION_MESSAGE_BYTES) {
        state.pending_server_events.push_back(envelope);
        return;
    }

    state.server_sequence = state.server_sequence.saturating_sub(1);
    let snapshot_id = request_id
        .clone()
        .unwrap_or_else(|| format!("snapshot-{}", state.server_sequence));
    let mut chunk_size = INITIAL_SNAPSHOT_CHUNK_PRIMS.max(1);

    loop {
        if snapshot.scene.prims.is_empty() {
            queue_compact_snapshot(state, request_id, snapshot, session_snapshot);
            return;
        }

        let chunks: Vec<ServerEventEnvelope> = snapshot
            .scene
            .prims
            .chunks(chunk_size)
            .enumerate()
            .map(|(chunk_index, prims)| {
                let mut chunk_state = snapshot.clone();
                chunk_state.scene.prims = prims.to_vec();
                let chunk_count = snapshot.scene.prims.len().div_ceil(chunk_size);
                next_server_envelope(
                    state,
                    request_id.as_deref(),
                    ServerEvent::Session(SessionEvent::SnapshotChunk {
                        snapshot_id: snapshot_id.clone(),
                        chunk_index: chunk_index as u32,
                        chunk_count: chunk_count as u32,
                        state: chunk_state,
                    }),
                )
            })
            .collect();

        if chunks.iter().all(|envelope| {
            encoded_size(envelope).is_some_and(|size| size <= MAX_APPLICATION_MESSAGE_BYTES)
        }) {
            info!(
                "[viewport-data-channel] queued snapshot {} in {} chunks ({} prims)",
                snapshot_id,
                chunks.len(),
                snapshot.scene.prims.len()
            );
            state.pending_server_events.extend(chunks);
            return;
        }

        state.server_sequence = state.server_sequence.saturating_sub(chunks.len() as u64);
        if chunk_size == 1 {
            let oversized_prims = chunks
                .iter()
                .map(|envelope| {
                    !encoded_size(envelope)
                        .is_some_and(|size| size <= MAX_APPLICATION_MESSAGE_BYTES)
                })
                .collect::<Vec<_>>();
            let omitted_count = oversized_prims
                .iter()
                .filter(|oversized| **oversized)
                .count();

            if omitted_count == 0 {
                error!(
                    "[viewport-data-channel] snapshot chunk sizing failed without an oversized prim; sending a compact snapshot"
                );
                queue_compact_snapshot(state, request_id, snapshot, session_snapshot);
                return;
            }

            snapshot.scene.prims = snapshot
                .scene
                .prims
                .into_iter()
                .zip(oversized_prims)
                .filter_map(|(prim, oversized)| (!oversized).then_some(prim))
                .collect();
            warn!(
                "[viewport-data-channel] omitted {omitted_count} prim(s) that exceed the application message limit"
            );
            chunk_size = INITIAL_SNAPSHOT_CHUNK_PRIMS.max(1);
            continue;
        }
        chunk_size = (chunk_size / 2).max(1);
    }
}

pub(super) fn queue_compact_snapshot(
    state: &mut ApplicationSessionState,
    request_id: Option<String>,
    mut snapshot: ViewportReadModel,
    session_snapshot: bool,
) {
    snapshot.scene.prims.clear();
    snapshot.selection.target = None;
    snapshot.camera_source = CameraSource::Arcball;
    snapshot.stage.display_name = truncate_snapshot_display_name(&snapshot.stage.display_name);

    if queue_bounded_event(
        state,
        request_id.as_deref(),
        snapshot_event(snapshot.clone(), session_snapshot),
    ) {
        warn!(
            "[viewport-data-channel] queued a compact snapshot after the full snapshot exceeded the application message limit"
        );
        return;
    }

    let mut minimal = ViewportReadModel::unloaded("remote-stage");
    minimal.stage.loaded = snapshot.stage.loaded;
    minimal.scene.total_prims = snapshot.scene.total_prims;
    minimal.scene.total_roots = snapshot.scene.total_roots;
    minimal.scene.root_page_size = snapshot.scene.root_page_size;
    minimal.timeline = snapshot.timeline;
    minimal.presentation = snapshot.presentation;
    minimal.physics_running = snapshot.physics_running;

    if !queue_bounded_event(state, None, snapshot_event(minimal, session_snapshot)) {
        error!(
            "[viewport-data-channel] failed to queue the bounded minimal snapshot after snapshot compaction"
        );
    }
}

pub(super) fn truncate_snapshot_display_name(display_name: &str) -> String {
    let truncated = display_name
        .chars()
        .take(MAX_COMPACT_STAGE_DISPLAY_NAME_CHARS)
        .collect::<String>();
    if truncated.len() == display_name.len() {
        truncated
    } else {
        format!("{truncated}…")
    }
}
