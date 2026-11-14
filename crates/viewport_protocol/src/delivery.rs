//! Authorized self-render manifests and content-addressed runtime payloads.
//!
//! These types describe what may cross the delivery boundary. They do not
//! expose filesystem paths or Bevy asset handles, and an authorized manifest
//! contains only blob references allowed by the session policy.

use serde::{Deserialize, Serialize};
use std::{collections::HashSet, error::Error, fmt};

use crate::{AuthorizationPolicy, RuntimeProfile};

const MAX_RUNTIME_REVISION_BYTES: usize = 256;
const MAX_RUNTIME_BLOB_REFERENCES: usize = 1_000_000;

/// A derived payload category in a self-render runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePayloadKind {
    Hierarchy,
    Mesh,
    Material,
    Texture,
}

/// Metadata needed to request and hydrate one content-addressed runtime blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeBlobReference {
    pub blob_id: String,
    pub payload_kind: RuntimePayloadKind,
    pub payload_version: u16,
    pub byte_size: u64,
}

impl RuntimeBlobReference {
    pub fn validate(&self) -> Result<(), RuntimeManifestValidationError> {
        validate_runtime_blob_id(&self.blob_id)?;
        if self.payload_version == 0 {
            return Err(RuntimeManifestValidationError::InvalidPayloadVersion);
        }
        Ok(())
    }
}

/// The server-side runtime inventory before session authorization filtering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeManifest {
    pub revision: String,
    pub profile: RuntimeProfile,
    pub hierarchy: RuntimeBlobReference,
    pub meshes: Vec<RuntimeBlobReference>,
    pub materials: Vec<RuntimeBlobReference>,
    pub textures: Vec<RuntimeBlobReference>,
}

impl RuntimeManifest {
    pub fn validate(&self) -> Result<(), RuntimeManifestValidationError> {
        if self.revision.trim().is_empty() {
            return Err(RuntimeManifestValidationError::EmptyRevision);
        }
        if self.revision.len() > MAX_RUNTIME_REVISION_BYTES {
            return Err(RuntimeManifestValidationError::RevisionTooLong);
        }

        let references = self.references();
        if references.len() > MAX_RUNTIME_BLOB_REFERENCES {
            return Err(RuntimeManifestValidationError::TooManyBlobReferences);
        }

        let mut seen = HashSet::with_capacity(references.len());
        for reference in references {
            reference.validate()?;
            if !seen.insert(reference.blob_id.as_str()) {
                return Err(RuntimeManifestValidationError::DuplicateBlobId(
                    reference.blob_id.clone(),
                ));
            }
        }
        Ok(())
    }

    /// Produces the only manifest shape that may be sent to a self-render
    /// client. Disallowed references are omitted, never sent for client-side
    /// hiding. The hierarchy is required because it is the local renderer's
    /// root runtime payload.
    pub fn authorize(
        &self,
        policy: &AuthorizationPolicy,
    ) -> Result<AuthorizedRuntimeManifest, RuntimeManifestAuthorizationError> {
        self.validate()
            .map_err(RuntimeManifestAuthorizationError::InvalidManifest)?;
        policy
            .validate()
            .map_err(RuntimeManifestAuthorizationError::InvalidPolicy)?;
        if !policy.allows_self_render_delivery() {
            return Err(RuntimeManifestAuthorizationError::SelfRenderNotAllowed);
        }
        if !policy.allows_model_download() {
            return Err(RuntimeManifestAuthorizationError::ModelDownloadNotAllowed);
        }

        let allowed =
            |reference: &RuntimeBlobReference| policy.allows_runtime_blob(&reference.blob_id);
        if !allowed(&self.hierarchy) {
            return Err(RuntimeManifestAuthorizationError::RequiredBlobNotAllowed(
                self.hierarchy.blob_id.clone(),
            ));
        }

        let mut redacted_blob_count = 0;
        let filter = |references: &[RuntimeBlobReference], count: &mut u32| {
            references
                .iter()
                .filter_map(|reference| {
                    if allowed(reference) {
                        Some(reference.clone())
                    } else {
                        *count = count.saturating_add(1);
                        None
                    }
                })
                .collect::<Vec<_>>()
        };

        Ok(AuthorizedRuntimeManifest {
            revision: self.revision.clone(),
            profile: policy.runtime_profile,
            hierarchy: self.hierarchy.clone(),
            meshes: filter(&self.meshes, &mut redacted_blob_count),
            materials: filter(&self.materials, &mut redacted_blob_count),
            textures: filter(&self.textures, &mut redacted_blob_count),
            redacted_blob_count,
        })
    }

    pub fn references(&self) -> Vec<&RuntimeBlobReference> {
        let mut references =
            Vec::with_capacity(1 + self.meshes.len() + self.materials.len() + self.textures.len());
        references.push(&self.hierarchy);
        references.extend(&self.meshes);
        references.extend(&self.materials);
        references.extend(&self.textures);
        references
    }
}

/// A manifest after policy filtering. Every blob reference in this value is
/// authorized for possession by the receiving session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizedRuntimeManifest {
    pub revision: String,
    pub profile: RuntimeProfile,
    pub hierarchy: RuntimeBlobReference,
    pub meshes: Vec<RuntimeBlobReference>,
    pub materials: Vec<RuntimeBlobReference>,
    pub textures: Vec<RuntimeBlobReference>,
    pub redacted_blob_count: u32,
}

impl AuthorizedRuntimeManifest {
    pub fn validate(&self) -> Result<(), RuntimeManifestValidationError> {
        let references = self.references();
        if references.len() > MAX_RUNTIME_BLOB_REFERENCES {
            return Err(RuntimeManifestValidationError::TooManyBlobReferences);
        }

        let mut seen = HashSet::with_capacity(references.len());
        for reference in references {
            reference.validate()?;
            if !seen.insert(reference.blob_id.as_str()) {
                return Err(RuntimeManifestValidationError::DuplicateBlobId(
                    reference.blob_id.clone(),
                ));
            }
        }
        Ok(())
    }

    pub fn allows_blob(&self, blob_id: &str) -> bool {
        self.references()
            .iter()
            .any(|reference| reference.blob_id == blob_id)
    }

    pub fn references(&self) -> Vec<&RuntimeBlobReference> {
        let mut references =
            Vec::with_capacity(1 + self.meshes.len() + self.materials.len() + self.textures.len());
        references.push(&self.hierarchy);
        references.extend(&self.meshes);
        references.extend(&self.materials);
        references.extend(&self.textures);
        references
    }
}

/// Validation failures for server-owned runtime manifests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeManifestValidationError {
    EmptyRevision,
    RevisionTooLong,
    InvalidBlobId(String),
    InvalidPayloadVersion,
    DuplicateBlobId(String),
    TooManyBlobReferences,
}

impl fmt::Display for RuntimeManifestValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRevision => formatter.write_str("runtime manifest revision is empty"),
            Self::RevisionTooLong => formatter.write_str("runtime manifest revision is too long"),
            Self::InvalidBlobId(blob_id) => {
                write!(formatter, "invalid runtime blob id {blob_id:?}")
            }
            Self::InvalidPayloadVersion => {
                formatter.write_str("runtime blob payload version must be greater than zero")
            }
            Self::DuplicateBlobId(blob_id) => {
                write!(formatter, "runtime manifest repeats blob id {blob_id:?}")
            }
            Self::TooManyBlobReferences => {
                formatter.write_str("runtime manifest contains too many blob references")
            }
        }
    }
}

impl Error for RuntimeManifestValidationError {}

/// Authorization failures that prevent a manifest from crossing the boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeManifestAuthorizationError {
    InvalidManifest(RuntimeManifestValidationError),
    InvalidPolicy(crate::AuthorizationValidationError),
    SelfRenderNotAllowed,
    ModelDownloadNotAllowed,
    RequiredBlobNotAllowed(String),
}

impl fmt::Display for RuntimeManifestAuthorizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManifest(error) => write!(formatter, "invalid runtime manifest: {error}"),
            Self::InvalidPolicy(error) => {
                write!(formatter, "invalid authorization policy: {error}")
            }
            Self::SelfRenderNotAllowed => {
                formatter.write_str("self-render delivery is not allowed")
            }
            Self::ModelDownloadNotAllowed => {
                formatter.write_str("runtime model download is not allowed")
            }
            Self::RequiredBlobNotAllowed(blob_id) => {
                write!(
                    formatter,
                    "required runtime blob {blob_id:?} is not authorized"
                )
            }
        }
    }
}

impl Error for RuntimeManifestAuthorizationError {}

/// Validates the canonical lowercase hexadecimal representation used by the
/// existing filesystem BlobStore and transport manifest.
pub fn validate_runtime_blob_id(blob_id: &str) -> Result<(), RuntimeManifestValidationError> {
    if blob_id.len() != 64
        || !blob_id
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
    {
        return Err(RuntimeManifestValidationError::InvalidBlobId(
            blob_id.to_owned(),
        ));
    }
    Ok(())
}
