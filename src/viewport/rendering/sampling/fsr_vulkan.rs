//! Renderer-local FidelityFX FSR2 provider contract and camera binding.
//!
//! The provider is deliberately target-gated. Bevy 0.19 does not ship a native
//! FSR provider, so Linux and Windows use the isolated Vulkan backend in
//! `backend.rs`; other targets remain fail-closed.

use bevy::prelude::*;

use super::coordinator::{ActiveUpscaler, SamplingCoordinatorState};

/// Runtime facts required before the coordinator may select FSR.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FsrVulkanCapability {
    pub(crate) vulkan_backend: bool,
    pub(crate) fidelityfx_backend: bool,
    pub(crate) input_contract_ready: bool,
}

impl Default for FsrVulkanCapability {
    fn default() -> Self {
        Self::from_probe(false, false, false)
    }
}

impl FsrVulkanCapability {
    pub(crate) const fn from_probe(
        vulkan_backend: bool,
        fidelityfx_backend: bool,
        input_contract_ready: bool,
    ) -> Self {
        Self {
            vulkan_backend,
            fidelityfx_backend,
            input_contract_ready,
        }
    }

    pub(crate) const fn supported(self) -> bool {
        self.vulkan_backend && self.fidelityfx_backend && self.input_contract_ready
    }
}

/// Renderer-only resources required by the selected FSR generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FsrFrameInput {
    pub(crate) input_extent: UVec2,
    pub(crate) output_extent: UVec2,
    pub(crate) motion_vectors: bool,
    pub(crate) depth: bool,
    pub(crate) exposure: bool,
    pub(crate) jitter: bool,
    pub(crate) camera_parameters: bool,
    pub(crate) cpu_readback: bool,
}

/// Explicit rejection reasons for incomplete provider integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FsrInputError {
    InvalidResolution,
    MissingMotionVectors,
    MissingDepth,
    MissingExposure,
    MissingJitter,
    MissingCameraParameters,
    CpuReadbackInPipeline,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FsrCameraParameters {
    pub(crate) near: f32,
    pub(crate) far: f32,
    pub(crate) fov_y: f32,
}

pub(crate) fn fsr_camera_parameters(
    projection: &bevy::camera::Projection,
) -> Option<FsrCameraParameters> {
    match projection {
        bevy::camera::Projection::Perspective(projection) => Some(FsrCameraParameters {
            near: projection.near,
            far: projection.far,
            fov_y: projection.fov,
        }),
        bevy::camera::Projection::Orthographic(_) | bevy::camera::Projection::Custom(_) => None,
    }
}

pub(crate) fn fsr_frame_delta_ms(delta_secs: f32) -> f32 {
    delta_secs.clamp(0.001, 0.25) * 1000.0
}

pub(crate) fn fsr_jitter_offset(frame_index: u32) -> Vec2 {
    const JITTER_PHASES: u32 = 18;
    let phase = frame_index % JITTER_PHASES + 1;
    Vec2::new(halton(phase, 2) - 0.5, halton(phase, 3) - 0.5)
}

fn halton(mut index: u32, base: u32) -> f32 {
    let mut result = 0.0;
    let mut fraction = 1.0 / base as f32;
    while index != 0 {
        result += fraction * (index % base) as f32;
        index /= base;
        fraction /= base as f32;
    }
    result
}

/// Isolated FSR adapter surface consumed by renderer code, never by protocol.
pub(crate) struct FsrVulkanProvider;

impl FsrVulkanProvider {
    pub(crate) fn validate_frame_input(input: FsrFrameInput) -> Result<(), FsrInputError> {
        if input.input_extent.x == 0
            || input.input_extent.y == 0
            || input.output_extent.x == 0
            || input.output_extent.y == 0
            || input.input_extent.x >= input.output_extent.x
            || input.input_extent.y >= input.output_extent.y
        {
            return Err(FsrInputError::InvalidResolution);
        }
        if !input.motion_vectors {
            return Err(FsrInputError::MissingMotionVectors);
        }
        if !input.depth {
            return Err(FsrInputError::MissingDepth);
        }
        if !input.exposure {
            return Err(FsrInputError::MissingExposure);
        }
        if !input.jitter {
            return Err(FsrInputError::MissingJitter);
        }
        if !input.camera_parameters {
            return Err(FsrInputError::MissingCameraParameters);
        }
        if input.cpu_readback {
            return Err(FsrInputError::CpuReadbackInPipeline);
        }
        Ok(())
    }
}

/// Registers the FSR capability and the camera input contract.
pub(crate) struct FsrVulkanProviderPlugin;

impl Plugin for FsrVulkanProviderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FsrVulkanCapability>().add_systems(
            Update,
            configure_fsr_camera
                .after(crate::viewport::api::ViewportBridgeSet::ApplyCommands)
                .before(crate::viewport::api::ViewportBridgeSet::ReduceEvents),
        );

        #[cfg(all(
            feature = "fsr_vulkan",
            any(target_os = "linux", target_os = "windows")
        ))]
        backend::build_vulkan_provider(app);
    }
}

#[derive(Component, Debug, Clone, Copy)]
struct FsrCameraBinding {
    added_depth: bool,
    added_motion_vectors: bool,
    added_texture_usages: bool,
    previous_texture_usages: Option<bevy::render::render_resource::TextureUsages>,
    added_temporal_jitter: bool,
    previous_temporal_jitter: Option<Vec2>,
}

fn configure_fsr_camera(
    coordinator: Res<SamplingCoordinatorState>,
    target: Option<Res<crate::viewport::app::headless::OffscreenTarget>>,
    frame_count: Option<Res<bevy::diagnostic::FrameCount>>,
    mut commands: Commands,
    cameras: Query<
        (
            Entity,
            Option<&bevy::camera::MainPassResolutionOverride>,
            Has<bevy::core_pipeline::prepass::DepthPrepass>,
            Has<bevy::core_pipeline::prepass::MotionVectorPrepass>,
            Option<&bevy::camera::CameraMainTextureUsages>,
            Option<&bevy::render::camera::TemporalJitter>,
            Option<&FsrCameraBinding>,
        ),
        With<Camera3d>,
    >,
) {
    let Some(target) = target else {
        return;
    };

    let output_extent = UVec2::new(target.width, target.height);
    let fsr_active = coordinator.active == ActiveUpscaler::Fsr;
    let input_extent = fsr_render_extent(output_extent);
    let jitter = fsr_jitter_offset(frame_count.map_or(0, |frame| frame.0));

    for (
        entity,
        resolution_override,
        has_depth,
        has_motion,
        texture_usages,
        temporal_jitter,
        binding,
    ) in &cameras
    {
        if fsr_active && input_extent != output_extent {
            if resolution_override.is_none_or(|current| current.0 != input_extent) {
                commands
                    .entity(entity)
                    .insert(bevy::camera::MainPassResolutionOverride(input_extent));
            }
            if !has_depth || !has_motion {
                commands.entity(entity).insert((
                    bevy::core_pipeline::prepass::DepthPrepass,
                    bevy::core_pipeline::prepass::MotionVectorPrepass,
                ));
            }
            let required_usages = bevy::render::render_resource::TextureUsages::STORAGE_BINDING
                | bevy::render::render_resource::TextureUsages::COPY_DST;
            if !texture_usages.is_some_and(|usages| usages.0.contains(required_usages)) {
                let usages = texture_usages.map_or(
                    bevy::render::render_resource::TextureUsages::RENDER_ATTACHMENT
                        | bevy::render::render_resource::TextureUsages::TEXTURE_BINDING
                        | bevy::render::render_resource::TextureUsages::COPY_SRC,
                    |usages| usages.0,
                );
                commands
                    .entity(entity)
                    .insert(bevy::camera::CameraMainTextureUsages(
                        usages | required_usages,
                    ));
            }
            commands
                .entity(entity)
                .insert(bevy::render::camera::TemporalJitter { offset: jitter });
            if binding.is_none() {
                commands.entity(entity).insert(FsrCameraBinding {
                    added_depth: !has_depth,
                    added_motion_vectors: !has_motion,
                    added_texture_usages: !texture_usages
                        .is_some_and(|usages| usages.0.contains(required_usages)),
                    previous_texture_usages: texture_usages.map(|usages| usages.0),
                    added_temporal_jitter: temporal_jitter.is_none(),
                    previous_temporal_jitter: temporal_jitter.map(|jitter| jitter.offset),
                });
            }
        } else if let Some(binding) = binding {
            let mut entity_commands = commands.entity(entity);
            entity_commands.remove::<bevy::camera::MainPassResolutionOverride>();
            if binding.added_depth {
                entity_commands.remove::<bevy::core_pipeline::prepass::DepthPrepass>();
            }
            if binding.added_motion_vectors {
                entity_commands.remove::<bevy::core_pipeline::prepass::MotionVectorPrepass>();
            }
            if binding.added_texture_usages {
                if let Some(previous) = binding.previous_texture_usages {
                    entity_commands.insert(bevy::camera::CameraMainTextureUsages(previous));
                } else {
                    entity_commands.remove::<bevy::camera::CameraMainTextureUsages>();
                }
            }
            if binding.added_temporal_jitter {
                entity_commands.remove::<bevy::render::camera::TemporalJitter>();
            } else if let Some(previous) = binding.previous_temporal_jitter {
                entity_commands.insert(bevy::render::camera::TemporalJitter { offset: previous });
            }
            entity_commands.remove::<FsrCameraBinding>();
        }
    }
}

fn fsr_render_extent(output_extent: UVec2) -> UVec2 {
    UVec2::new(
        ((output_extent.x as f32 * 0.667).floor() as u32)
            .max(1)
            .min(output_extent.x.saturating_sub(1).max(1)),
        ((output_extent.y as f32 * 0.667).floor() as u32)
            .max(1)
            .min(output_extent.y.saturating_sub(1).max(1)),
    )
}

#[cfg(all(
    feature = "fsr_vulkan",
    any(target_os = "linux", target_os = "windows")
))]
mod backend;

#[cfg(test)]
#[path = "fsr_vulkan_tests.rs"]
mod tests;
