//! Atomic, target-aware Project runtime-cache descriptors.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use usd_model::HashDigest;
use uuid::Uuid;
use viewport_protocol::{RuntimeManifest, RuntimeProfile};

use super::target_content_hash;
use crate::project::cache_contract::ProjectCacheTarget;
use crate::project::{catalog::manifest_store::write_bytes_atomic, storage::ProjectStorageLayout};

pub(crate) const PROJECT_CACHE_DESCRIPTOR_SCHEMA_VERSION: u16 = 2;
const DESCRIPTORS_DIRECTORY: &str = "descriptors";

/// All source and runtime choices that must agree before a descriptor is
/// reusable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ProjectCacheIdentity {
    pub(crate) target: ProjectCacheTarget,
    pub(crate) target_content_hash: HashDigest,
    pub(crate) profile: RuntimeProfile,
    pub(crate) config_hash: HashDigest,
}

impl ProjectCacheIdentity {
    pub(crate) fn for_project(
        project_root: &Path,
        target: ProjectCacheTarget,
        profile: RuntimeProfile,
        config_hash: HashDigest,
    ) -> Result<Self> {
        let target_content_hash = target_content_hash(project_root, &target)?;
        Ok(Self {
            target: target.clone(),
            target_content_hash,
            profile,
            config_hash,
        })
    }
}

/// Descriptor state exposed internally to the Project service and, later,
/// reduced to a small cache status in the UI protocol.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProjectCacheState {
    Empty,
    Building,
    Ready,
    Partial,
    FallbackRequired,
}

/// One atomically published cache index record. The descriptor is published
/// after its referenced blobs are complete; it never owns authoritative USD.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ProjectCacheDescriptor {
    pub(crate) schema_version: u16,
    pub(crate) identity: ProjectCacheIdentity,
    pub(crate) state: ProjectCacheState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) runtime: Option<RuntimeManifest>,
}

impl ProjectCacheDescriptor {
    pub(crate) fn new(
        identity: ProjectCacheIdentity,
        state: ProjectCacheState,
        runtime: Option<RuntimeManifest>,
    ) -> Result<Self> {
        let descriptor = Self {
            schema_version: PROJECT_CACHE_DESCRIPTOR_SCHEMA_VERSION,
            identity,
            state,
            runtime,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == PROJECT_CACHE_DESCRIPTOR_SCHEMA_VERSION,
            "unsupported Project cache descriptor schema version {}",
            self.schema_version
        );
        if let Some(runtime) = &self.runtime {
            runtime
                .validate()
                .map_err(|error| anyhow::anyhow!("invalid cached runtime manifest: {error}"))?;
        }
        ensure!(
            self.state != ProjectCacheState::Ready || self.runtime.is_some(),
            "ready Project cache descriptor must include a runtime manifest"
        );
        Ok(())
    }
}

/// Filesystem owner for cache descriptors under .usdhub/cache.
#[derive(Clone, Debug)]
pub(crate) struct ProjectCacheStore {
    descriptors: PathBuf,
}

impl ProjectCacheStore {
    pub(crate) fn new(project_root: &Path) -> Self {
        Self {
            descriptors: ProjectStorageLayout::new(project_root)
                .cache_dir()
                .join(DESCRIPTORS_DIRECTORY),
        }
    }

    pub(crate) fn descriptor_path(&self, identity: &ProjectCacheIdentity) -> Result<PathBuf> {
        let bytes = serde_json::to_vec(identity).context("encode Project cache identity")?;
        let key = blake3::hash(&bytes).to_hex().to_string();
        Ok(self.descriptors.join(format!("{}.json", key)))
    }

    pub(crate) fn publish(&self, descriptor: &ProjectCacheDescriptor) -> Result<PathBuf> {
        descriptor.validate()?;
        fs::create_dir_all(&self.descriptors).with_context(|| {
            format!("create Project cache index {}", self.descriptors.display())
        })?;
        let path = self.descriptor_path(&descriptor.identity)?;
        let bytes =
            serde_json::to_vec_pretty(descriptor).context("encode Project cache descriptor")?;
        let temporary = self
            .descriptors
            .join(format!(".descriptor.{}.tmp", Uuid::new_v4()));
        write_bytes_atomic(&temporary, &path, &bytes)
            .context("publish Project cache descriptor")?;
        Ok(path)
    }

    pub(crate) fn load(
        &self,
        identity: &ProjectCacheIdentity,
    ) -> Result<Option<ProjectCacheDescriptor>> {
        let path = self.descriptor_path(identity)?;
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let descriptor: ProjectCacheDescriptor = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode Project cache descriptor {}", path.display()))?;
        ensure!(
            descriptor.identity == *identity,
            "Project cache descriptor identity does not match its lookup"
        );
        descriptor.validate()?;
        Ok(Some(descriptor))
    }

    /// Remove every derived descriptor for one deleted Project target while
    /// leaving content-addressed payload objects available for reuse.
    pub(crate) fn remove_target(&self, target: &ProjectCacheTarget) -> Result<usize> {
        let entries = match fs::read_dir(&self.descriptors) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "read Project cache descriptors {}",
                        self.descriptors.display()
                    )
                });
            }
        };
        let mut removed = 0;
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            let descriptor: ProjectCacheDescriptor = match serde_json::from_slice(&bytes) {
                Ok(descriptor) => descriptor,
                Err(_) => continue,
            };
            if descriptor.identity.target == *target {
                fs::remove_file(path)?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/project/cache_descriptor_tests.rs"]
mod tests;
