use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{AuthorizedRuntimeManifest, SessionEvent};

use super::types::{HydratedRuntimeDelivery, RuntimeDeliveryClientError, RuntimeDeliveryUpdate};

const MAX_RUNTIME_CHUNKS: u32 = 1_000_000;

#[derive(Debug)]
struct ManifestChunkAssembly {
    manifest_id: String,
    chunk_count: u32,
    chunks: BTreeMap<u32, AuthorizedRuntimeManifest>,
}

#[derive(Debug)]
struct BlobChunkAssembly {
    chunk_count: u32,
    chunks: BTreeMap<u32, Vec<u8>>,
    received_bytes: u64,
}

/// Accumulates runtime manifests and content-addressed blobs until a full
/// authorized payload has been verified.
#[derive(Debug, Default)]
pub struct RuntimeDeliveryAssembler {
    manifest: Option<AuthorizedRuntimeManifest>,
    manifest_chunks: Option<ManifestChunkAssembly>,
    blobs: BTreeMap<String, Vec<u8>>,
    blob_chunks: HashMap<String, BlobChunkAssembly>,
}

impl RuntimeDeliveryAssembler {
    /// Applies a session event emitted by the reliable viewport channel.
    pub fn apply_session_event(
        &mut self,
        event: &SessionEvent,
    ) -> Result<RuntimeDeliveryUpdate, RuntimeDeliveryClientError> {
        match event {
            SessionEvent::AuthorizationChanged { authorization } => {
                self.clear();
                Ok(RuntimeDeliveryUpdate::AuthorizationChanged {
                    authorization: authorization.clone(),
                })
            }
            SessionEvent::RuntimeManifest { manifest } => {
                self.accept_manifest(manifest.clone())?;
                Ok(RuntimeDeliveryUpdate::ManifestAccepted)
            }
            SessionEvent::RuntimeManifestChunk {
                manifest_id,
                chunk_index,
                chunk_count,
                manifest,
            } => {
                let complete = self.accept_manifest_chunk(
                    manifest_id,
                    *chunk_index,
                    *chunk_count,
                    manifest.clone(),
                )?;
                Ok(RuntimeDeliveryUpdate::ManifestChunkAccepted { complete })
            }
            SessionEvent::RuntimeBlobChunk {
                blob_id,
                chunk_index,
                chunk_count,
                bytes,
            } => {
                let complete =
                    self.accept_blob_chunk(blob_id, *chunk_index, *chunk_count, bytes.clone())?;
                Ok(RuntimeDeliveryUpdate::BlobChunkAccepted {
                    blob_id: blob_id.clone(),
                    complete,
                })
            }
            SessionEvent::RuntimeBlobRejected { reason } => {
                Ok(RuntimeDeliveryUpdate::BlobRejected {
                    reason: reason.clone(),
                })
            }
            _ => Ok(RuntimeDeliveryUpdate::Ignored),
        }
    }

    /// Installs an unchunked manifest and atomically starts a new hydration
    /// revision. Existing bytes are not carried across revisions.
    pub fn accept_manifest(
        &mut self,
        manifest: AuthorizedRuntimeManifest,
    ) -> Result<(), RuntimeDeliveryClientError> {
        manifest
            .validate()
            .map_err(RuntimeDeliveryClientError::InvalidManifest)?;
        self.manifest = Some(manifest);
        self.manifest_chunks = None;
        self.blobs.clear();
        self.blob_chunks.clear();
        Ok(())
    }

    /// Accepts one manifest chunk. Chunks may arrive out of order, but a
    /// conflicting duplicate is rejected and the assembled manifest is
    /// installed only after every declared chunk is present.
    pub fn accept_manifest_chunk(
        &mut self,
        manifest_id: &str,
        chunk_index: u32,
        chunk_count: u32,
        manifest: AuthorizedRuntimeManifest,
    ) -> Result<bool, RuntimeDeliveryClientError> {
        validate_chunk_coordinates(chunk_index, chunk_count)?;
        if manifest_id.trim().is_empty() {
            return Err(RuntimeDeliveryClientError::EmptyManifestId);
        }
        manifest
            .validate()
            .map_err(RuntimeDeliveryClientError::InvalidManifest)?;

        let assembly = match self.manifest_chunks.as_mut() {
            Some(assembly) if assembly.manifest_id == manifest_id => {
                if assembly.chunk_count != chunk_count {
                    return Err(RuntimeDeliveryClientError::ManifestChunkCountChanged);
                }
                assembly
            }
            Some(_) | None => {
                self.manifest_chunks = Some(ManifestChunkAssembly {
                    manifest_id: manifest_id.to_owned(),
                    chunk_count,
                    chunks: BTreeMap::new(),
                });
                self.manifest_chunks
                    .as_mut()
                    .expect("manifest chunk assembly was just inserted")
            }
        };

        if let Some(existing) = assembly.chunks.get(&chunk_index) {
            if existing != &manifest {
                return Err(RuntimeDeliveryClientError::ManifestChunkConflict);
            }
        } else {
            assembly.chunks.insert(chunk_index, manifest);
        }

        if assembly.chunks.len() != assembly.chunk_count as usize {
            return Ok(false);
        }

        let merged = merge_manifest_chunks(&assembly.chunks)?;
        self.manifest_chunks = None;
        self.accept_manifest(merged)?;
        Ok(true)
    }

    /// Accepts one blob chunk only when the blob was named by the installed
    /// authorized manifest. Completion verifies the declared byte size.
    pub fn accept_blob_chunk(
        &mut self,
        blob_id: &str,
        chunk_index: u32,
        chunk_count: u32,
        bytes: Vec<u8>,
    ) -> Result<bool, RuntimeDeliveryClientError> {
        validate_chunk_coordinates(chunk_index, chunk_count)?;
        crate::validate_runtime_blob_id(blob_id)
            .map_err(RuntimeDeliveryClientError::InvalidManifest)?;
        let expected = self
            .manifest
            .as_ref()
            .ok_or(RuntimeDeliveryClientError::ManifestRequired)?
            .references()
            .into_iter()
            .find(|reference| reference.blob_id == blob_id)
            .cloned()
            .ok_or_else(|| RuntimeDeliveryClientError::BlobNotAuthorized(blob_id.to_owned()))?;

        if self.blobs.contains_key(blob_id) {
            return Err(RuntimeDeliveryClientError::BlobAlreadyComplete(
                blob_id.to_owned(),
            ));
        }

        let assembly = self
            .blob_chunks
            .entry(blob_id.to_owned())
            .or_insert_with(|| BlobChunkAssembly {
                chunk_count,
                chunks: BTreeMap::new(),
                received_bytes: 0,
            });
        if assembly.chunk_count != chunk_count {
            return Err(RuntimeDeliveryClientError::BlobChunkCountChanged);
        }

        if let Some(existing) = assembly.chunks.get(&chunk_index) {
            if existing != &bytes {
                return Err(RuntimeDeliveryClientError::BlobChunkConflict);
            }
        } else {
            let received_bytes = assembly.received_bytes.saturating_add(bytes.len() as u64);
            if received_bytes > expected.byte_size {
                return Err(RuntimeDeliveryClientError::BlobSizeExceeded {
                    blob_id: blob_id.to_owned(),
                    expected: expected.byte_size,
                    received: received_bytes,
                });
            }
            assembly.received_bytes = received_bytes;
            assembly.chunks.insert(chunk_index, bytes);
        }

        if assembly.chunks.len() != assembly.chunk_count as usize {
            return Ok(false);
        }

        let received = assembly.received_bytes;
        if received != expected.byte_size {
            return Err(RuntimeDeliveryClientError::BlobSizeMismatch {
                blob_id: blob_id.to_owned(),
                expected: expected.byte_size,
                received,
            });
        }
        let assembled = assembly
            .chunks
            .values()
            .flat_map(|chunk| chunk.iter().copied())
            .collect::<Vec<_>>();
        self.blob_chunks.remove(blob_id);
        self.blobs.insert(blob_id.to_owned(), assembled);
        Ok(true)
    }

    pub fn manifest(&self) -> Option<&AuthorizedRuntimeManifest> {
        self.manifest.as_ref()
    }

    pub fn is_ready(&self) -> bool {
        let Some(manifest) = &self.manifest else {
            return false;
        };
        manifest
            .references()
            .into_iter()
            .all(|reference| self.blobs.contains_key(&reference.blob_id))
    }

    pub fn hydrated(&self) -> Option<HydratedRuntimeDelivery> {
        self.is_ready().then(|| HydratedRuntimeDelivery {
            manifest: self
                .manifest
                .as_ref()
                .expect("ready delivery always has a manifest")
                .clone(),
            blobs: self.blobs.clone(),
        })
    }

    pub fn clear(&mut self) {
        self.manifest = None;
        self.manifest_chunks = None;
        self.blobs.clear();
        self.blob_chunks.clear();
    }
}

fn validate_chunk_coordinates(
    chunk_index: u32,
    chunk_count: u32,
) -> Result<(), RuntimeDeliveryClientError> {
    if chunk_count == 0 || chunk_count > MAX_RUNTIME_CHUNKS {
        return Err(RuntimeDeliveryClientError::InvalidChunkCount);
    }
    if chunk_index >= chunk_count {
        return Err(RuntimeDeliveryClientError::ChunkIndexOutOfRange);
    }
    Ok(())
}

fn merge_manifest_chunks(
    chunks: &BTreeMap<u32, AuthorizedRuntimeManifest>,
) -> Result<AuthorizedRuntimeManifest, RuntimeDeliveryClientError> {
    let first = chunks
        .values()
        .next()
        .expect("manifest assembly cannot complete without chunks");
    let mut seen = HashSet::new();
    let mut meshes = Vec::new();
    let mut materials = Vec::new();
    let mut textures = Vec::new();

    for chunk in chunks.values() {
        if chunk.revision != first.revision
            || chunk.profile != first.profile
            || chunk.hierarchy != first.hierarchy
            || chunk.redacted_blob_count != first.redacted_blob_count
        {
            return Err(RuntimeDeliveryClientError::ManifestMetadataMismatch);
        }
        for reference in chunk
            .meshes
            .iter()
            .chain(&chunk.materials)
            .chain(&chunk.textures)
        {
            if !seen.insert(reference.blob_id.as_str()) {
                return Err(RuntimeDeliveryClientError::DuplicateBlobId(
                    reference.blob_id.clone(),
                ));
            }
        }
        meshes.extend(chunk.meshes.iter().cloned());
        materials.extend(chunk.materials.iter().cloned());
        textures.extend(chunk.textures.iter().cloned());
    }

    let merged = AuthorizedRuntimeManifest {
        revision: first.revision.clone(),
        profile: first.profile,
        hierarchy: first.hierarchy.clone(),
        meshes,
        materials,
        textures,
        redacted_blob_count: first.redacted_blob_count,
    };
    merged
        .validate()
        .map_err(RuntimeDeliveryClientError::InvalidManifest)?;
    Ok(merged)
}
