//! Target-gated FidelityFX/Vulkan render integration.

use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResourcePlugin;
use bevy::render::{ExtractSchedule, RenderApp, RenderStartup};

use super::coordinator::{ActiveUpscaler, SamplingCoordinatorState};
use super::{FsrFrameInput, FsrVulkanCapability, FsrVulkanProvider};

pub(super) fn build_vulkan_provider(app: &mut App) {
    app.add_plugins(ExtractResourcePlugin::<SamplingCoordinatorState>::default());

    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };

    render_app
        .init_non_send_resource::<FsrVulkanRenderBackend>()
        .add_systems(RenderStartup, initialize_vulkan_backend)
        .add_systems(ExtractSchedule, publish_backend_capability)
        .add_systems(
            Core3d,
            dispatch_fsr_vulkan.in_set(Core3dSystems::EarlyPostProcess),
        );
}

#[derive(Default)]
struct FsrVulkanRenderBackend {
    context: Option<fsr::Context>,
    display_size: [u32; 2],
    max_render_size: [u32; 2],
    attempted: bool,
    ready: bool,
}

fn initialize_vulkan_backend(mut backend: NonSendMut<FsrVulkanRenderBackend>) {
    backend.attempted = false;
}

fn publish_backend_capability(
    backend: NonSend<FsrVulkanRenderBackend>,
    mut main_world: ResMut<bevy::render::MainWorld>,
) {
    if !backend.attempted {
        return;
    }
    let Some(mut capability) = main_world.get_resource_mut::<FsrVulkanCapability>() else {
        return;
    };
    let next = FsrVulkanCapability::from_probe(backend.ready, backend.ready, backend.ready);
    if *capability != next {
        *capability = next;
    }
}

fn dispatch_fsr_vulkan(
    mut backend: NonSendMut<FsrVulkanRenderBackend>,
    instance: Res<bevy::render::renderer::RenderInstance>,
    adapter: Res<bevy::render::renderer::RenderAdapter>,
    render_device: Res<bevy::render::renderer::RenderDevice>,
    sampling: Res<SamplingCoordinatorState>,
    view: bevy::render::renderer::ViewQuery<(
        &bevy::render::view::ViewTarget,
        Option<&bevy::core_pipeline::prepass::ViewPrepassTextures>,
    )>,
    mut render_context: bevy::render::renderer::RenderContext,
) {
    if sampling.active != ActiveUpscaler::Fsr {
        return;
    }

    let (target, prepass) = view.into_inner();
    let Some(prepass) = prepass else {
        return;
    };
    let Some(depth) = prepass.depth.as_ref() else {
        return;
    };
    let Some(motion_vectors) = prepass.motion_vectors.as_ref() else {
        return;
    };

    let input_extent = UVec2::new(prepass.size.width, prepass.size.height);
    let output_extent = UVec2::new(
        target.main_texture().width(),
        target.main_texture().height(),
    );
    let input = FsrFrameInput {
        input_extent,
        output_extent,
        motion_vectors: true,
        depth: true,
        // The FSR context uses the SDK's auto-exposure path. This keeps the
        // exposure input on the GPU and avoids a CPU readback or staging image.
        exposure: true,
        cpu_readback: false,
    };
    if FsrVulkanProvider::validate_frame_input(input).is_err() {
        return;
    }

    if backend
        .ensure_context(
            &instance,
            &adapter,
            &render_device,
            output_extent,
            input_extent,
        )
        .is_err()
    {
        return;
    }
    let Some(context) = backend.context.as_mut() else {
        return;
    };

    let post_process = target.post_process_write();
    // SAFETY: all four resources are borrowed from this render invocation and
    // were validated against the live FSR context below.
    let Some(color) = (unsafe {
        vulkan_texture_resource(
            context,
            post_process.source_texture,
            post_process.source,
            fsr::ResourceStates::COMPUTE_READ,
            "bevy_fsr_color",
        )
    }) else {
        return;
    };
    // SAFETY: the prepass attachment is a live Vulkan texture/view pair from
    // the same render device as the FSR context.
    let Some(depth) = (unsafe {
        vulkan_texture_resource(
            context,
            &depth.texture.texture,
            &depth.texture.default_view,
            fsr::ResourceStates::COMPUTE_READ,
            "bevy_fsr_depth",
        )
    }) else {
        return;
    };
    // SAFETY: the motion-vector attachment is a live Vulkan texture/view pair
    // from the same render device as the FSR context.
    let Some(motion_vectors) = (unsafe {
        vulkan_texture_resource(
            context,
            &motion_vectors.texture.texture,
            &motion_vectors.texture.default_view,
            fsr::ResourceStates::COMPUTE_READ,
            "bevy_fsr_motion_vectors",
        )
    }) else {
        return;
    };
    // SAFETY: the post-process destination is a live storage-capable Vulkan
    // texture/view pair from the same render device as the FSR context.
    let Some(output) = (unsafe {
        vulkan_texture_resource(
            context,
            post_process.destination_texture,
            post_process.destination,
            fsr::ResourceStates::UNORDERED_ACCESS,
            "bevy_fsr_output",
        )
    }) else {
        return;
    };

    // SAFETY: the encoder is owned by Bevy's current render context; the raw
    // command buffer is borrowed only for this FidelityFX dispatch and is not
    // ended or destroyed by this provider.
    let command_buffer = render_context
        .command_encoder()
        .as_hal_mut::<wgpu::hal::api::Vulkan, _>(|encoder| {
            encoder.map(|encoder| {
                // SAFETY: wgpu will end and submit this encoder after the
                // current render schedule; FidelityFX only records into it.
                unsafe { encoder.raw_handle() }
            })
        });
    let Some(command_buffer) = command_buffer else {
        return;
    };

    let dispatch = fsr::DispatchDescription::new(
        command_buffer.into(),
        color,
        depth,
        motion_vectors,
        output,
        1.0 / 60.0,
        [input_extent.x, input_extent.y],
    )
    .reset(false);

    // SAFETY: the dispatch resources and command buffer all belong to the
    // same live Vulkan device and remain valid for the encoded operation.
    if unsafe { context.dispatch(dispatch) }.is_err() {
        render_context.command_encoder().copy_texture_to_texture(
            post_process.source_texture.as_image_copy(),
            post_process.destination_texture.as_image_copy(),
            post_process.source_texture.size(),
        );
        backend.ready = false;
    } else {
        backend.ready = true;
    }
}

impl FsrVulkanRenderBackend {
    fn ensure_context(
        &mut self,
        instance: &bevy::render::renderer::RenderInstance,
        adapter: &bevy::render::renderer::RenderAdapter,
        render_device: &bevy::render::renderer::RenderDevice,
        display_extent: UVec2,
        input_extent: UVec2,
    ) -> Result<(), ()> {
        let display_size = [display_extent.x, display_extent.y];
        let max_render_size = [input_extent.x, input_extent.y];
        if self.context.is_some()
            && (self.display_size != display_size || self.max_render_size != max_render_size)
        {
            if let Some(mut context) = self.context.take() {
                // SAFETY: the context is retired on the render thread before
                // the next context is created for the same Vulkan device.
                let _ = unsafe { context.destroy() };
            }
        }
        if self.context.is_some() {
            return Ok(());
        }

        self.attempted = true;
        // SAFETY: the Bevy instance remains alive for the entire render app.
        let Some(instance) = (unsafe { instance.as_hal::<wgpu::hal::api::Vulkan>() }) else {
            self.ready = false;
            return Err(());
        };
        // SAFETY: the Bevy adapter remains alive for the entire render app.
        let Some(adapter) = (unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }) else {
            self.ready = false;
            return Err(());
        };
        // SAFETY: the Bevy logical device remains alive for the entire render
        // app and owns the textures and command buffers used by the provider.
        let Some(device) = (unsafe {
            render_device
                .wgpu_device()
                .as_hal::<wgpu::hal::api::Vulkan>()
        }) else {
            self.ready = false;
            return Err(());
        };

        // SAFETY: these handles are borrowed from Bevy's live Vulkan HAL
        // instance and adapter; FidelityFX only stores the resulting
        // interface inside the context while those Bevy resources remain live.
        let interface = unsafe {
            fsr::vk::get_interface(
                instance.shared_instance().entry(),
                instance.shared_instance().raw_instance(),
                adapter.raw_physical_device(),
            )
        }
        .map_err(|_| ())?;
        // SAFETY: the cloned ash device handle refers to the same live logical
        // device owned by Bevy and is used only to create FidelityFX's opaque
        // device value; it is never destroyed by the adapter.
        let fsr_device = unsafe { fsr::vk::get_device(device.raw_device().clone()) };
        // SAFETY: the interface and device describe the same live Vulkan
        // instance/device pair, and all context dimensions are non-zero.
        let context = unsafe {
            fsr::Context::new(fsr::ContextDescription {
                interface,
                flags: fsr::InitializationFlagBits::ENABLE_HIGH_DYNAMIC_RANGE
                    | fsr::InitializationFlagBits::ENABLE_AUTO_EXPOSURE,
                max_render_size,
                display_size,
                device: &fsr_device,
                message_callback: None,
            })
        }
        .map_err(|_| ())?;

        self.context = Some(context);
        self.display_size = display_size;
        self.max_render_size = max_render_size;
        self.ready = true;
        Ok(())
    }
}

impl Drop for FsrVulkanRenderBackend {
    fn drop(&mut self) {
        if let Some(mut context) = self.context.take() {
            // SAFETY: this resource is dropped with the render world after
            // Bevy has stopped submitting work to the render device.
            let _ = unsafe { context.destroy() };
        }
    }
}

/// # Safety
///
/// The caller must keep `texture` and `view` alive and on the same Vulkan
/// device as `context` for the duration of the returned FidelityFX resource's
/// use in the current command buffer.
unsafe fn vulkan_texture_resource(
    context: &mut fsr::Context,
    texture: &wgpu::Texture,
    view: &wgpu::TextureView,
    state: fsr::ResourceStates,
    name: &str,
) -> Option<fsr::Resource> {
    // SAFETY: the borrowed wgpu objects are alive for this call and are not
    // used through wgpu while their HAL guards are inspected.
    let image = unsafe { texture.as_hal::<wgpu::hal::api::Vulkan>() }?;
    // SAFETY: the borrowed wgpu view is alive for this call and belongs to the
    // texture passed above.
    let image_view = unsafe { view.as_hal::<wgpu::hal::api::Vulkan>() }?;
    let format = vulkan_format(texture.format())?;
    // SAFETY: the HAL image/view pair belongs to the same live Vulkan device
    // as the FidelityFX context, and the dimensions/format are read directly
    // from the wgpu texture descriptor.
    Some(unsafe {
        fsr::vk::get_texture_resource(
            context,
            image.raw_handle(),
            image_view.raw_handle(),
            format,
            [texture.width(), texture.height()],
            state,
            name,
        )
    })
}

fn vulkan_format(format: wgpu::TextureFormat) -> Option<ash::vk::Format> {
    Some(match format {
        wgpu::TextureFormat::Rgba16Float => ash::vk::Format::R16G16B16A16_SFLOAT,
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => {
            ash::vk::Format::R8G8B8A8_UNORM
        }
        wgpu::TextureFormat::Rg16Float => ash::vk::Format::R16G16_SFLOAT,
        wgpu::TextureFormat::R32Float => ash::vk::Format::R32_SFLOAT,
        wgpu::TextureFormat::Depth32Float => ash::vk::Format::D32_SFLOAT,
        _ => return None,
    })
}
