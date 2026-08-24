use bevy::prelude::UVec2;

use super::{
    FsrFrameInput, FsrInputError, FsrVulkanCapability, FsrVulkanProvider, fsr_render_extent,
};

const VALID_INPUT: FsrFrameInput = FsrFrameInput {
    input_extent: UVec2::new(960, 540),
    output_extent: UVec2::new(1920, 1080),
    motion_vectors: true,
    depth: true,
    exposure: true,
    cpu_readback: false,
};

#[test]
fn capability_requires_every_runtime_boundary() {
    assert_eq!(
        FsrVulkanCapability::default().supported(),
        cfg!(all(
            feature = "fsr_vulkan",
            any(target_os = "linux", target_os = "windows")
        ))
    );
    assert!(FsrVulkanCapability::from_probe(true, true, true).supported());
    assert!(!FsrVulkanCapability::from_probe(true, true, false).supported());
}

#[test]
fn camera_render_extent_uses_a_strictly_lower_fsr_input_size() {
    assert_eq!(
        fsr_render_extent(UVec2::new(1920, 1080)),
        UVec2::new(1280, 720)
    );
}

#[test]
fn valid_input_has_lower_render_extent_and_no_cpu_readback() {
    assert_eq!(FsrVulkanProvider::validate_frame_input(VALID_INPUT), Ok(()));
}

#[test]
fn missing_temporal_inputs_fail_closed() {
    let cases = [
        (
            FsrFrameInput {
                motion_vectors: false,
                ..VALID_INPUT
            },
            FsrInputError::MissingMotionVectors,
        ),
        (
            FsrFrameInput {
                depth: false,
                ..VALID_INPUT
            },
            FsrInputError::MissingDepth,
        ),
        (
            FsrFrameInput {
                exposure: false,
                ..VALID_INPUT
            },
            FsrInputError::MissingExposure,
        ),
        (
            FsrFrameInput {
                cpu_readback: true,
                ..VALID_INPUT
            },
            FsrInputError::CpuReadbackInPipeline,
        ),
    ];

    for (input, expected) in cases {
        assert_eq!(
            FsrVulkanProvider::validate_frame_input(input),
            Err(expected)
        );
    }
}

#[test]
fn equal_or_invalid_extents_are_not_upscaling() {
    for input_extent in [UVec2::ZERO, UVec2::new(1920, 1080), UVec2::new(2000, 540)] {
        let input = FsrFrameInput {
            input_extent,
            ..VALID_INPUT
        };
        assert_eq!(
            FsrVulkanProvider::validate_frame_input(input),
            Err(FsrInputError::InvalidResolution)
        );
    }
}
