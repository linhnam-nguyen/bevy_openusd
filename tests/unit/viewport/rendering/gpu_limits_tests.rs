use bevy::render::settings::WgpuLimits;

use super::GpuLimitNegotiation;

#[test]
fn deformation_storage_limits_fit_the_adapter_and_preserve_host_requirements() {
    for (available, existing, expected, expected_status) in [
        (0, 0, 0, GpuLimitNegotiation::BelowPreferredFloor { supported: 0 }),
        (
            0,
            16,
            0,
            GpuLimitNegotiation::ReducedUnsupportedRequest {
                prior: 16,
                supported: 0,
            },
        ),
        (4, 4, 4, GpuLimitNegotiation::BelowPreferredFloor { supported: 4 }),
        (8, 8, 8, GpuLimitNegotiation::BelowPreferredFloor { supported: 8 }),
        (
            8,
            32,
            8,
            GpuLimitNegotiation::ReducedUnsupportedRequest {
                prior: 32,
                supported: 8,
            },
        ),
        (9, 8, 9, GpuLimitNegotiation::BelowPreferredFloor { supported: 9 }),
        (
            15,
            32,
            15,
            GpuLimitNegotiation::ReducedUnsupportedRequest {
                prior: 32,
                supported: 15,
            },
        ),
        (16, 8, 16, GpuLimitNegotiation::Supported(16)),
        (16, 16, 16, GpuLimitNegotiation::Supported(16)),
        (
            16,
            32,
            16,
            GpuLimitNegotiation::ReducedUnsupportedRequest {
                prior: 32,
                supported: 16,
            },
        ),
        (32, 8, 16, GpuLimitNegotiation::Supported(16)),
        (32, 24, 24, GpuLimitNegotiation::Supported(24)),
        (
            32,
            40,
            32,
            GpuLimitNegotiation::ReducedUnsupportedRequest {
                prior: 40,
                supported: 32,
            },
        ),
    ] {
        let supported = WgpuLimits {
            max_storage_buffers_per_shader_stage: available,
            ..Default::default()
        };
        let mut requested = WgpuLimits {
            max_storage_buffers_per_shader_stage: existing,
            max_sampled_textures_per_shader_stage: 32,
            ..Default::default()
        };
        let mut wanted = requested.clone();
        wanted.max_storage_buffers_per_shader_stage = expected;
        let status = super::configure(&supported, &mut requested);
        assert_eq!(
            status, expected_status,
            "unexpected status for available={available}, existing={existing}"
        );
        assert_eq!(
            requested, wanted,
            "failed for available={available}, existing={existing}"
        );
        assert!(requested.max_storage_buffers_per_shader_stage <= available);
    }
}

#[test]
fn deformation_storage_limits_are_idempotent_and_preserve_unrelated_limits() {
    let supported = WgpuLimits {
        max_storage_buffers_per_shader_stage: 15,
        max_sampled_textures_per_shader_stage: 8,
        ..Default::default()
    };
    let mut requested = WgpuLimits {
        max_storage_buffers_per_shader_stage: 32,
        max_sampled_textures_per_shader_stage: 24,
        max_uniform_buffers_per_shader_stage: 12,
        ..Default::default()
    };

    let first = super::configure(&supported, &mut requested);
    assert_eq!(
        first,
        GpuLimitNegotiation::ReducedUnsupportedRequest {
            prior: 32,
            supported: 15,
        }
    );
    assert_eq!(requested.max_storage_buffers_per_shader_stage, 15);

    let second = super::configure(&supported, &mut requested);
    assert_eq!(
        second,
        GpuLimitNegotiation::BelowPreferredFloor { supported: 15 }
    );
    assert_eq!(requested.max_storage_buffers_per_shader_stage, 15);
    assert_eq!(requested.max_sampled_textures_per_shader_stage, 24);
    assert_eq!(requested.max_uniform_buffers_per_shader_stage, 12);
}
