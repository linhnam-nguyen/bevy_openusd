use std::{collections::BTreeMap, error::Error, fmt};

use crate::{AuthorizationPolicy, AuthorizedRuntimeManifest, RuntimeManifestValidationError};

/// A complete authorized runtime that is ready for local hydration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydratedRuntimeDelivery {
    pub manifest: AuthorizedRuntimeManifest,
    pub(super) blobs: BTreeMap<String, Vec<u8>>,
}

impl HydratedRuntimeDelivery {
    /// Returns verified bytes for a blob named by the authorized manifest.
    pub fn blob(&self, blob_id: &str) -> Option<&[u8]> {
        self.blobs.get(blob_id).map(Vec::as_slice)
    }

    /// Returns the verified blob set in deterministic identifier order.
    pub fn blobs(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.blobs
            .iter()
            .map(|(blob_id, bytes)| (blob_id.as_str(), bytes.as_slice()))
    }
}

/// Progress reported after applying one runtime delivery event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeDeliveryUpdate {
    Ignored,
    ManifestAccepted,
    ManifestChunkAccepted { complete: bool },
    BlobChunkAccepted { blob_id: String, complete: bool },
    BlobRejected { reason: String },
    AuthorizationChanged { authorization: AuthorizationPolicy },
}

/// Errors raised while assembling an authorized runtime bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeDeliveryClientError {
    InvalidManifest(RuntimeManifestValidationError),
    EmptyManifestId,
    InvalidChunkCount,
    ChunkIndexOutOfRange,
    ManifestChunkCountChanged,
    ManifestChunkConflict,
    ManifestMetadataMismatch,
    DuplicateBlobId(String),
    ManifestRequired,
    BlobNotAuthorized(String),
    BlobChunkCountChanged,
    BlobChunkConflict,
    BlobAlreadyComplete(String),
    BlobSizeExceeded {
        blob_id: String,
        expected: u64,
        received: u64,
    },
    BlobSizeMismatch {
        blob_id: String,
        expected: u64,
        received: u64,
    },
}

impl fmt::Display for RuntimeDeliveryClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManifest(error) => {
                write!(formatter, "invalid authorized manifest: {error}")
            }
            Self::EmptyManifestId => formatter.write_str("runtime manifest id is empty"),
            Self::InvalidChunkCount => formatter.write_str("runtime chunk count is invalid"),
            Self::ChunkIndexOutOfRange => {
                formatter.write_str("runtime chunk index is out of range")
            }
            Self::ManifestChunkCountChanged => {
                formatter.write_str("runtime manifest chunk count changed mid-assembly")
            }
            Self::ManifestChunkConflict => {
                formatter.write_str("runtime manifest chunk was received with conflicting data")
            }
            Self::ManifestMetadataMismatch => {
                formatter.write_str("runtime manifest chunks disagree on revision or profile")
            }
            Self::DuplicateBlobId(blob_id) => {
                write!(formatter, "duplicate blob id in manifest: {blob_id}")
            }
            Self::ManifestRequired => formatter.write_str("manifest must precede runtime blobs"),
            Self::BlobNotAuthorized(blob_id) => {
                write!(formatter, "blob is not in authorized manifest: {blob_id}")
            }
            Self::BlobChunkCountChanged => {
                formatter.write_str("blob chunk count changed mid-assembly")
            }
            Self::BlobChunkConflict => {
                formatter.write_str("blob chunk was received with conflicting payload bytes")
            }
            Self::BlobAlreadyComplete(blob_id) => {
                write!(formatter, "blob is already complete: {blob_id}")
            }
            Self::BlobSizeExceeded {
                blob_id,
                expected,
                received,
            } => write!(
                formatter,
                "blob {blob_id} exceeded declared byte size ({received} > {expected})"
            ),
            Self::BlobSizeMismatch {
                blob_id,
                expected,
                received,
            } => write!(
                formatter,
                "blob {blob_id} assembled size did not match manifest ({received} != {expected})"
            ),
        }
    }
}

impl Error for RuntimeDeliveryClientError {}
