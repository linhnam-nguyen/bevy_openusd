//! Identity context shared by cache hydration and activation lifecycle code.

use std::path::PathBuf;

use anyhow::Result;
use bevy::prelude::Resource;
use viewport_protocol::RuntimeProfile;

use crate::project::cache::{ProjectCacheIdentity, ProjectCacheTarget};

/// The exact cache identity attached to the currently active Project stage.
/// It is backend-only and is never exposed to the frontend or renderer crate.
#[derive(Clone, Debug, Resource)]
pub(crate) struct ActiveProjectCacheContext {
    pub(crate) project_root: PathBuf,
    pub(crate) identity: ProjectCacheIdentity,
    revalidate_on_activation: bool,
}

impl ActiveProjectCacheContext {
    pub(crate) fn new(
        project_root: PathBuf,
        target: ProjectCacheTarget,
        profile: RuntimeProfile,
        config_hash: usd_model::HashDigest,
    ) -> Result<Self> {
        let identity =
            ProjectCacheIdentity::for_project(&project_root, target, profile, config_hash)?;
        Ok(Self {
            project_root,
            identity,
            revalidate_on_activation: true,
        })
    }

    pub(crate) fn from_identity(project_root: PathBuf, identity: ProjectCacheIdentity) -> Self {
        Self {
            project_root,
            identity,
            revalidate_on_activation: false,
        }
    }

    pub(crate) fn should_revalidate(&self) -> bool {
        self.revalidate_on_activation
    }
}

/// The application-wide semantic configuration used for runtime cache
/// identity. Keeping this in one helper prevents warm and delivery paths from
/// accidentally publishing different configuration hashes.
pub(crate) fn default_project_cache_config_hash() -> usd_model::HashDigest {
    crate::project::cache_compatibility::project_runtime_cache_config_hash(
        usd_semantic::SemanticConfig::default().hash(),
    )
}
