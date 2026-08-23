//! Renderer policy for selecting a sampling provider.
//!
//! Capability probing and provider activation are intentionally separate from
//! this module. B4.1 only defines the deterministic policy that consumes the
//! probe result; later checkpoints will supply the actual Vulkan capabilities
//! and provider implementations.

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
