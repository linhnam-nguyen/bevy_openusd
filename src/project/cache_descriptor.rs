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
use crate::project::{catalog::manifest_store::write_bytes_atomic, storage::ProjectStorageLayout};

pub(crate) const PROJECT_CACHE_DESCRIPTOR_SCHEMA_VERSION: u16 = 2;
const DESCRIPTORS_DIRECTORY: &str = "descriptors";

/// Stable Project content target used in a cache identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum ProjectCacheTarget {
    ProjectRoot,
    Scene { id: String },
    Model { id: String },
}

impl ProjectCacheTarget {
    pub(crate) fn key(&self) -> String {
        match self {
            Self::ProjectRoot => "project".to_owned(),
            Self::Scene { id } => format!("scene:{}", id),
            Self::Model { id } => format!("model:{}", id),
        }
    }
}

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
mod tests {
    use tempfile::tempdir;
    use usd_model::HashDigest;

    use super::*;

    fn make_identity(id: &str) -> ProjectCacheIdentity {
        ProjectCacheIdentity {
            target: ProjectCacheTarget::Scene { id: id.to_owned() },
            target_content_hash: HashDigest::new([6; HashDigest::BYTE_LEN]),
            profile: RuntimeProfile::NativeMedium,
            config_hash: HashDigest::new([7; HashDigest::BYTE_LEN]),
        }
    }

    #[test]
    fn descriptor_round_trip_is_atomic_and_target_specific() -> Result<()> {
        let directory = tempdir()?;
        usd_git::Repository::init(directory.path())?;
        let store = ProjectCacheStore::new(directory.path());
        let identity = make_identity("scene-a");
        let descriptor =
            ProjectCacheDescriptor::new(identity.clone(), ProjectCacheState::Partial, None)?;

        let path = store.publish(&descriptor)?;
        assert!(path.is_file());
        assert_eq!(store.load(&identity)?, Some(descriptor));
        assert_eq!(fs::read_dir(path.parent().unwrap())?.count(), 1);

        let other = make_identity("scene-b");
        assert_ne!(
            store.descriptor_path(&identity)?,
            store.descriptor_path(&other)?
        );
        Ok(())
    }

    #[test]
    fn corrupt_descriptor_is_rejected_without_source_fallback_data() -> Result<()> {
        let directory = tempdir()?;
        usd_git::Repository::init(directory.path())?;
        let store = ProjectCacheStore::new(directory.path());
        let identity = make_identity("scene-a");
        let path = store.descriptor_path(&identity)?;
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, b"not-json")?;

        assert!(store.load(&identity).is_err());
        Ok(())
    }

    #[test]
    fn stale_descriptor_schema_is_rejected_and_config_identity_is_a_miss() -> Result<()> {
        let directory = tempdir()?;
        usd_git::Repository::init(directory.path())?;
        let store = ProjectCacheStore::new(directory.path());
        let identity = make_identity("scene-a");
        let descriptor =
            ProjectCacheDescriptor::new(identity.clone(), ProjectCacheState::Partial, None)?;
        let path = store.publish(&descriptor)?;

        let mut stale = serde_json::to_value(&descriptor)?;
        stale["schema_version"] = serde_json::json!(999);
        fs::write(&path, serde_json::to_vec(&stale)?)?;
        assert!(store.load(&identity).is_err());

        store.publish(&descriptor)?;
        let different_config = ProjectCacheIdentity {
            config_hash: HashDigest::new([8; HashDigest::BYTE_LEN]),
            ..identity.clone()
        };
        assert!(store.load(&different_config)?.is_none());
        Ok(())
    }

    #[test]
    fn descriptors_reuse_when_target_content_is_unchanged() -> Result<()> {
        let directory = tempdir()?;
        let store = ProjectCacheStore::new(directory.path());
        let target = ProjectCacheTarget::ProjectRoot;
        let identity_a = ProjectCacheIdentity {
            target: target.clone(),
            target_content_hash: HashDigest::new([9; HashDigest::BYTE_LEN]),
            profile: RuntimeProfile::NativeMedium,
            config_hash: HashDigest::new([7; HashDigest::BYTE_LEN]),
        };
        let identity_b = ProjectCacheIdentity {
            ..identity_a.clone()
        };
        store.publish(&ProjectCacheDescriptor::new(
            identity_a.clone(),
            ProjectCacheState::Partial,
            None,
        )?)?;

        assert!(store.load(&identity_a)?.is_some());
        assert!(store.load(&identity_b)?.is_some());
        assert_eq!(
            store.descriptor_path(&identity_a)?,
            store.descriptor_path(&identity_b)?
        );
        Ok(())
    }

    #[test]
    fn removing_a_target_drops_descriptors_but_keeps_the_object_store() -> Result<()> {
        let directory = tempdir()?;
        usd_git::Repository::init(directory.path())?;
        let store = ProjectCacheStore::new(directory.path());
        let target = ProjectCacheTarget::Scene {
            id: "scene-a".to_owned(),
        };
        let identity = ProjectCacheIdentity {
            target_content_hash: HashDigest::new([1; HashDigest::BYTE_LEN]),
            target: target.clone(),
            profile: RuntimeProfile::NativeMedium,
            config_hash: HashDigest::new([2; HashDigest::BYTE_LEN]),
        };
        store.publish(&ProjectCacheDescriptor::new(
            identity.clone(),
            ProjectCacheState::Partial,
            None,
        )?)?;
        assert_eq!(store.remove_target(&target)?, 1);
        assert!(store.load(&identity)?.is_none());
        Ok(())
    }
}
