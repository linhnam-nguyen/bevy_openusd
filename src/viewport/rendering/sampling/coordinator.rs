//! Renderer policy for selecting a sampling provider.
//!
//! Capability probing and provider activation remain separate from this
//! coordinator. The coordinator owns the deterministic policy and the
//! renderer-local selection state consumed by provider adapters.

use bevy::prelude::Resource;
use viewport_protocol::SamplingProvider;

/// The renderer-selected upscaler, with `None` meaning sampling is disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActiveUpscaler {
    None,
    Dlss,
    Fsr,
}

/// Renderer capabilities used by the sampling policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SamplingCapabilities {
    dlss: bool,
    fsr: bool,
}

impl SamplingCapabilities {
    pub(crate) const fn new(dlss: bool, fsr: bool) -> Self {
        Self { dlss, fsr }
    }
}

/// Authoritative renderer-local sampling selection.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SamplingCoordinatorState {
    pub(crate) preference_enabled: bool,
    pub(crate) active: ActiveUpscaler,
}

impl bevy::render::extract_resource::ExtractResource for SamplingCoordinatorState {
    type Source = Self;

    fn extract_resource(source: &Self) -> Self {
        *source
    }
}

impl Default for SamplingCoordinatorState {
    fn default() -> Self {
        Self {
            preference_enabled: false,
            active: ActiveUpscaler::None,
        }
    }
}

impl ActiveUpscaler {
    pub(crate) const fn provider(self) -> SamplingProvider {
        match self {
            Self::None => SamplingProvider::None,
            Self::Dlss => SamplingProvider::Dlss,
            Self::Fsr => SamplingProvider::Fsr,
        }
    }
}

impl SamplingCoordinatorState {
    pub(crate) fn apply(&mut self, preference_enabled: bool, active: ActiveUpscaler) {
        self.preference_enabled = preference_enabled;
        self.active = active;
    }
}

/// Explicit failure when sampling was requested but no provider is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SamplingSelectionError {
    NoProviderAvailable,
}

/// Selects the preferred renderer provider without activating any provider.
///
/// DLSS has priority when both providers are available. A disabled preference
/// never requires capabilities, while an enabled preference with no provider
/// returns an error so callers cannot silently claim that sampling is active.
pub(crate) const fn choose_upscaler(
    preference_enabled: bool,
    capabilities: SamplingCapabilities,
) -> Result<ActiveUpscaler, SamplingSelectionError> {
    if !preference_enabled {
        return Ok(ActiveUpscaler::None);
    }

    if capabilities.dlss {
        Ok(ActiveUpscaler::Dlss)
    } else if capabilities.fsr {
        Ok(ActiveUpscaler::Fsr)
    } else {
        Err(SamplingSelectionError::NoProviderAvailable)
    }
}
