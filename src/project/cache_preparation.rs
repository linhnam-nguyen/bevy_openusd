use std::{path::Path, time::Duration};

#[cfg(test)]
use anyhow::Result;
#[cfg(test)]
use std::time::Instant;
use viewport_protocol::RuntimeProfile;

#[cfg(test)]
use super::ProjectCacheDescriptor;
use super::{ProjectCacheIdentity, ProjectCacheState, ProjectCacheTarget, ProjectCacheWarmQueue};

const CACHE_PREPARATION_POLL: Duration = Duration::from_millis(5);

/// Bounded result used by activation preparation before the Bevy world is touched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProjectCachePreparation {
    Ready,
    Empty,
    FallbackRequired,
}

impl ProjectCacheWarmQueue {
    /// Probe an exact target cache without making it part of activation.
    ///
    /// Cache warming is advisory. A miss, partial descriptor, or in-flight
    /// build immediately selects canonical source projection; the mutation
    /// paths remain responsible for scheduling background warms.
    pub(crate) fn prepare_for_activation(
        &self,
        project_root: &Path,
        target: ProjectCacheTarget,
    ) -> ProjectCachePreparation {
        let store = super::ProjectCacheStore::new(project_root);
        let identity = match ProjectCacheIdentity::for_project(
            project_root,
            target,
            RuntimeProfile::NativeMedium,
            super::super::cache_compatibility::project_runtime_cache_config_hash(
                usd_semantic::SemanticConfig::default().hash(),
            ),
        ) {
            Ok(identity) => identity,
            Err(error) => {
                log::warn!(
                    "Project cache activation identity could not be established for {}: {error:#}",
                    project_root.display()
                );
                return ProjectCachePreparation::FallbackRequired;
            }
        };
        Self::probe_for_activation(&store, &identity)
    }

    /// Inspect one already-computed identity. This deliberately performs no
    /// queue operation and no wait, so the caller can carry the identity into
    /// the main-world activation without hashing the target again.
    pub(crate) fn probe_for_activation(
        store: &super::ProjectCacheStore,
        identity: &ProjectCacheIdentity,
    ) -> ProjectCachePreparation {
        match store.load(identity) {
            Ok(Some(descriptor)) => match descriptor.state {
                ProjectCacheState::Ready => ProjectCachePreparation::Ready,
                ProjectCacheState::Empty => ProjectCachePreparation::Empty,
                ProjectCacheState::FallbackRequired
                | ProjectCacheState::Building
                | ProjectCacheState::Partial => ProjectCachePreparation::FallbackRequired,
            },
            Ok(None) => ProjectCachePreparation::FallbackRequired,
            Err(error) => {
                log::warn!(
                    "Project cache activation descriptor is unavailable for {}: {error:#}",
                    identity.target.key()
                );
                ProjectCachePreparation::FallbackRequired
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn wait_for(
    _queue: &ProjectCacheWarmQueue,
    project_root: &Path,
    target: &ProjectCacheTarget,
) -> Result<Option<ProjectCacheDescriptor>> {
    let identity = ProjectCacheIdentity::for_project(
        project_root,
        target.clone(),
        RuntimeProfile::NativeMedium,
        super::super::cache_compatibility::project_runtime_cache_config_hash(
            usd_semantic::SemanticConfig::default().hash(),
        ),
    )?;
    let store = super::ProjectCacheStore::new(project_root);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(descriptor) = store.load(&identity)? {
            if descriptor.state != ProjectCacheState::Building {
                return Ok(Some(descriptor));
            }
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(CACHE_PREPARATION_POLL);
    }
}
