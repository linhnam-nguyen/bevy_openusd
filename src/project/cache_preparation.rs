use std::{
    path::Path,
    time::{Duration, Instant},
};

#[cfg(test)]
use anyhow::Result;
use usd_semantic::SemanticConfig;
use viewport_protocol::RuntimeProfile;

#[cfg(test)]
use super::ProjectCacheDescriptor;
use super::{ProjectCacheIdentity, ProjectCacheState, ProjectCacheTarget, ProjectCacheWarmQueue};

const CACHE_PREPARATION_TIMEOUT: Duration = Duration::from_secs(10);
const CACHE_PREPARATION_POLL: Duration = Duration::from_millis(5);

/// Bounded result used by activation preparation before the Bevy world is touched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProjectCachePreparation {
    Ready,
    Empty,
    FallbackRequired,
    TimedOut,
}

impl ProjectCacheWarmQueue {
    /// Ensure an exact target cache is ready before activation, or return a
    /// bounded source-fallback outcome without blocking the Bevy owner thread.
    pub(crate) fn prepare_for_activation(
        &self,
        project_root: &Path,
        target: ProjectCacheTarget,
    ) -> ProjectCachePreparation {
        let deadline = Instant::now() + CACHE_PREPARATION_TIMEOUT;
        let store = super::ProjectCacheStore::new(project_root);
        let mut requested_identity = None;

        loop {
            let identity = match ProjectCacheIdentity::for_project(
                project_root,
                target.clone(),
                RuntimeProfile::NativeMedium,
                SemanticConfig::default().hash(),
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
            if requested_identity.as_ref() != Some(&identity) {
                requested_identity = None;
            }

            match store.load(&identity) {
                Ok(Some(descriptor)) => match descriptor.state {
                    ProjectCacheState::Ready => return ProjectCachePreparation::Ready,
                    ProjectCacheState::Empty => return ProjectCachePreparation::Empty,
                    ProjectCacheState::FallbackRequired => {
                        return ProjectCachePreparation::FallbackRequired;
                    }
                    ProjectCacheState::Building => {}
                    ProjectCacheState::Partial => {
                        if requested_identity.is_none() {
                            if !self.enqueue(project_root, target.clone()) {
                                return ProjectCachePreparation::TimedOut;
                            }
                            requested_identity = Some(identity.clone());
                        }
                    }
                },
                Ok(None) => {
                    if requested_identity.is_none() {
                        if !self.enqueue(project_root, target.clone()) {
                            return ProjectCachePreparation::TimedOut;
                        }
                        requested_identity = Some(identity.clone());
                    }
                }
                Err(error) => {
                    log::warn!(
                        "Project cache activation descriptor is unavailable for {}: {error:#}",
                        project_root.display()
                    );
                    return ProjectCachePreparation::FallbackRequired;
                }
            }

            if Instant::now() >= deadline {
                return ProjectCachePreparation::TimedOut;
            }
            std::thread::sleep(CACHE_PREPARATION_POLL);
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
        SemanticConfig::default().hash(),
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
