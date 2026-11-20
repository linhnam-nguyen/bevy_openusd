//! Wgpu storage-buffer limit negotiation for skinned/morphed meshes.
//!
//! Bevy 0.20 defaults to `WgpuSettingsPriority::Functionality`, which requests
//! the adapter's full supported limits; this helper enforces the explicit
//! 16-storage-buffer floor (clamped to adapter support) for custom limit hooks
//! (such as Mara `WindowApp::configure_gpu_limits` in UP-C02).

use bevy::render::settings::WgpuLimits;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GpuLimitNegotiation {
    Supported(u32),
    ReducedUnsupportedRequest { prior: u32, supported: u32 },
    BelowPreferredFloor { supported: u32 },
}

pub(crate) fn configure(supported: &WgpuLimits, requested: &mut WgpuLimits) -> GpuLimitNegotiation {
    let cap = supported.max_storage_buffers_per_shader_stage;
    let prior = requested.max_storage_buffers_per_shader_stage;
    let desired = cap.min(16);
    let bounded_prior = prior.min(cap);
    let next = bounded_prior.max(desired);
    debug_assert!(next <= cap);
    requested.max_storage_buffers_per_shader_stage = next;
    if prior > cap {
        GpuLimitNegotiation::ReducedUnsupportedRequest {
            prior,
            supported: cap,
        }
    } else if cap < 16 {
        GpuLimitNegotiation::BelowPreferredFloor { supported: cap }
    } else {
        GpuLimitNegotiation::Supported(next)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/rendering/gpu_limits_tests.rs"]
mod tests;
