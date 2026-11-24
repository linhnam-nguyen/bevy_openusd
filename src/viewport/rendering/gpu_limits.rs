//! Wgpu storage-buffer limit negotiation for skinned/morphed meshes.
//!
//! Bevy 0.20 defaults to `WgpuSettingsPriority::Functionality`, which requests
//! the adapter's full supported limits; this helper enforces the explicit
//! 16-storage-buffer floor (clamped to adapter support) for custom limit hooks
//! (such as Mara `WindowApp::configure_gpu_limits` in UP-C02).

use bevy::core_pipeline::oit::{
    OitFragmentNode, OrderIndependentTransparencySettings, resolve::OIT_REQUIRED_STORAGE_BUFFERS,
};
use bevy::prelude::*;
use bevy::render::{
    RenderApp, RenderStartup,
    error_handler::{RenderError, RenderErrorHandler, RenderErrorPolicy},
    mesh::allocator::MeshAllocatorSettings,
    renderer::RenderDevice,
    settings::WgpuLimits,
};

#[derive(Resource, Default)]
struct RendererStopped(bool);

pub(super) fn install_render_guards(app: &mut App) {
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render.add_systems(RenderStartup, bound_mesh_slabs);
    app.init_resource::<RendererStopped>()
        .insert_resource(RenderErrorHandler(stop_rendering))
        .add_systems(PreUpdate, guard_transparency_msaa)
        .add_systems(Last, guard_transparency_buffers);
}

fn guard_transparency_msaa(
    mut commands: Commands,
    cameras: Query<(Entity, &Msaa), With<OrderIndependentTransparencySettings>>,
) {
    for (entity, msaa) in &cameras {
        if msaa.samples() > 1 {
            commands
                .entity(entity)
                .remove::<OrderIndependentTransparencySettings>();
            warn!("Disabled order-independent transparency on an MSAA camera");
        }
    }
}

fn stop_rendering(error: &RenderError, world: &mut World, _: &mut World) -> RenderErrorPolicy {
    let mut stopped = world.resource_mut::<RendererStopped>();
    if !stopped.0 {
        stopped.0 = true;
        error!(
            "Renderer stopped ({:?}). Save any edits before restarting the viewer.",
            error.ty
        );
    }
    RenderErrorPolicy::StopRendering
}

fn bound_mesh_slabs(
    device: Res<RenderDevice>,
    mut settings: Option<ResMut<MeshAllocatorSettings>>,
) {
    let Some(settings) = settings.as_deref_mut() else {
        return;
    };
    let maximum = (device.limits().max_buffer_size / 2).max(1);
    settings.max_slab_size = settings.max_slab_size.min(maximum);
    settings.min_slab_size = settings.min_slab_size.min(settings.max_slab_size);
    settings.large_threshold = settings.large_threshold.min(settings.max_slab_size);
}

fn transparency_buffer_bytes(size: UVec2, average: f32) -> Option<u64> {
    let pixels = u64::from(size.x) * u64::from(size.y);
    let nodes = (pixels as f32 * average).ceil();
    if pixels > u64::from(u32::MAX)
        || !average.is_finite()
        || average <= 0.0
        || f64::from(nodes) > f64::from(u32::MAX)
    {
        return None;
    }
    Some((nodes as u64 * size_of::<OitFragmentNode>() as u64).max(pixels * 4))
}

fn guard_transparency_buffers(
    mut commands: Commands,
    device: Option<Res<RenderDevice>>,
    cameras: Query<(Entity, &Camera, &OrderIndependentTransparencySettings)>,
) {
    let Some(device) = device else {
        return;
    };
    let limits = device.limits();
    let mut size = UVec2::ZERO;
    let mut average = 0.0f32;
    let mut active = Vec::new();
    for (entity, camera, settings) in &cameras {
        if camera.is_active
            && let Some(target) = camera.physical_target_size()
        {
            let fragments = settings.fragments_per_pixel_average;
            if !fragments.is_finite() || fragments <= 0.0 {
                commands
                    .entity(entity)
                    .remove::<OrderIndependentTransparencySettings>();
                warn!("Disabled order-independent transparency with invalid fragment budget");
                continue;
            }
            size = size.max(target);
            average = average.max(fragments);
            active.push(entity);
        }
    }
    let limit = limits
        .max_buffer_size
        .min(limits.max_storage_buffer_binding_size);
    if !active.is_empty()
        && (limits.max_storage_buffers_per_shader_stage < OIT_REQUIRED_STORAGE_BUFFERS
            || transparency_buffer_bytes(size, average).is_none_or(|bytes| bytes > limit))
    {
        warn!(
            "Disabled order-independent transparency for {size:?} render targets at a {limit}-byte storage limit"
        );
        for entity in active {
            commands
                .entity(entity)
                .remove::<OrderIndependentTransparencySettings>();
        }
    }
}

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
