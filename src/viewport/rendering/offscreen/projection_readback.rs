//! Positive GPU completion of the current Project's initial offscreen projection.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::asset::{AssetId, UntypedAssetId};
use bevy::camera::{Camera, NormalizedRenderTarget, RenderTarget};
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::{
    MeshMaterial3d, PreparedMaterial, RenderMaterialInstances, RenderMeshInstances,
    SpecializedMaterialPipelineCache, StandardMaterial,
};
use bevy::prelude::{
    App, Entity, Image, Last, Local, NonSend, Or, Query, Res, ResMut, Resource, Update, With, World,
};
use bevy::render::camera::ExtractedCamera;
use bevy::render::erased_render_asset::ErasedRenderAssets;
use bevy::render::mesh::RenderMesh;
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d, MapMode, PipelineCache,
    TexelCopyBufferInfo, TexelCopyBufferLayout, TextureFormat,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, RenderVisibleEntities, ViewTarget};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderSystems};
use usd_bevy::{LiveStage, ProgressiveProjectionState, ProjectionReadiness, UsdPrimRef};
use viewport_protocol::InitialGpuProjection;

use super::OffscreenTarget;
use crate::viewport::navigation::camera::ArcballCamera;
use crate::viewport::rendering::residency::activation_preview::{
    WarmPreviewMesh, WarmScenePreview,
};
use crate::viewport::rendering::residency::{SceneResidencyOccurrence, VisualStreamingHealth};
use crate::viewport::stage::{StageHandle, StageInfo};

/// Time allowed for a rendered receipt once the CPU-side initial projection is ready.
const GPU_RECEIPT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProjectionIdentity {
    activation: u64,
    stage_session: u64,
    stage_revision: u64,
    projection: u64,
    target: u64,
}

#[derive(Default)]
struct CurrentReadback {
    identity: Option<ProjectionIdentity>,
    completion: Option<InitialGpuProjection>,
}

#[derive(Clone, Default, Resource)]
struct ProjectionReadback(Arc<Mutex<CurrentReadback>>);

#[derive(Default, Resource, Clone)]
struct PreviewReadback(ProjectionReadback);

impl ProjectionReadback {
    fn observe(&self, identity: ProjectionIdentity) {
        let Ok(mut current) = self.0.lock() else {
            return;
        };
        if current.identity != Some(identity) {
            current.identity = Some(identity);
            current.completion = None;
        }
    }

    fn status(&self, identity: ProjectionIdentity) -> InitialGpuProjection {
        let Ok(current) = self.0.lock() else {
            return InitialGpuProjection::Failed {
                message: "Initial GPU projection receipt is unavailable.".to_owned(),
            };
        };
        if current.identity == Some(identity) {
            current
                .completion
                .clone()
                .unwrap_or(InitialGpuProjection::Pending)
        } else {
            InitialGpuProjection::Pending
        }
    }

    fn reset(&self) {
        if let Ok(mut current) = self.0.lock() {
            *current = CurrentReadback::default();
        }
    }

    fn complete(&self, identity: ProjectionIdentity) {
        self.finish(identity, InitialGpuProjection::Complete);
    }

    fn finish(&self, identity: ProjectionIdentity, status: InitialGpuProjection) {
        let Ok(mut current) = self.0.lock() else {
            return;
        };
        if current.identity == Some(identity) && current.completion.is_none() {
            current.completion = Some(status);
        }
    }

    fn is_complete(&self, identity: ProjectionIdentity) -> bool {
        self.status(identity) == InitialGpuProjection::Complete
    }
}

#[derive(Default, Resource)]
struct ExtractedProjection {
    identity: Option<ProjectionIdentity>,
    output: Option<AssetId<Image>>,
    camera: Option<Entity>,
    preview: bool,
    assets: Vec<ProjectionAsset>,
}

struct ProjectionAsset {
    mesh: AssetId<Mesh>,
    material: Option<UntypedAssetId>,
}

#[derive(Default, Resource)]
struct ProjectionCaptureRequest {
    identity: Option<ProjectionIdentity>,
    preview_camera: Option<Entity>,
}

pub(crate) fn install(app: &mut App) {
    let receipt = ProjectionReadback::default();
    let preview_receipt = PreviewReadback::default();
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render
        .insert_resource(receipt.clone())
        .insert_resource(preview_receipt.clone())
        .init_resource::<ExtractedProjection>()
        .add_systems(ExtractSchedule, extract_projection)
        .add_systems(
            Render,
            submit_projection_readback.in_set(RenderSystems::Cleanup),
        );
    app.insert_resource(receipt)
        .insert_resource(preview_receipt)
        .init_resource::<ProjectionCaptureRequest>()
        .add_systems(
            Update,
            observe_projection
                .before(crate::viewport::ViewportFrameSet::PublishStageLoadState)
                .after(usd_bevy::LiveStageSet::Project),
        )
        .add_systems(Last, observe_projection);
}

fn preview_identity(preview: &WarmScenePreview, target: &OffscreenTarget) -> ProjectionIdentity {
    ProjectionIdentity {
        activation: preview.activation,
        stage_session: 0,
        stage_revision: 0,
        projection: preview.camera.to_bits(),
        target: target.generation,
    }
}

pub(crate) fn completed_preview(world: &World, activation: u64) -> bool {
    world
        .get_resource::<WarmScenePreview>()
        .zip(world.get_resource::<OffscreenTarget>())
        .zip(world.get_resource::<PreviewReadback>())
        .is_some_and(|((preview, target), receipt)| {
            preview.activation == activation
                && receipt.0.is_complete(preview_identity(preview, target))
        })
}

fn observe_projection(
    stage: Option<NonSend<LiveStage>>,
    stage_handle: Option<Res<StageHandle>>,
    projection: Option<Res<ProgressiveProjectionState>>,
    target: Option<Res<OffscreenTarget>>,
    health: Option<Res<VisualStreamingHealth>>,
    receipt: Res<ProjectionReadback>,
    preview: Option<Res<WarmScenePreview>>,
    preview_receipt: Res<PreviewReadback>,
    stage_info: Option<ResMut<StageInfo>>,
    mut capture: ResMut<ProjectionCaptureRequest>,
    mut started: Local<Option<(ProjectionIdentity, Instant)>>,
) {
    // Every observation owns the next capture request, including early exits.
    *capture = ProjectionCaptureRequest::default();
    if let (Some(preview), Some(target)) = (preview.as_deref(), target.as_deref()) {
        let identity = preview_identity(preview, target);
        preview_receipt.0.observe(identity);
        capture.identity = (preview_receipt.0.status(identity) == InitialGpuProjection::Pending)
            .then_some(identity);
        capture.preview_camera = Some(preview.camera);
        return;
    }
    preview_receipt.0.reset();
    let (Some(stage), Some(projection), Some(target), Some(mut stage_info)) =
        (stage, projection, target, stage_info)
    else {
        receipt.reset();
        *started = None;
        return;
    };
    if stage_info.activation_generation == 0 {
        stage_info.initial_gpu_projection = None;
        receipt.reset();
        *started = None;
        return;
    }
    // Stage installation owns the initial result. Later projection revisions,
    // residency changes and target resizes are not new Project activations.
    // Only a pending activation may replace its in-flight receipt identity.
    if !matches!(
        stage_info.initial_gpu_projection.as_ref(),
        Some(InitialGpuProjection::Pending)
    ) {
        *started = None;
        return;
    }
    let current_projection = projection.session_id() == Some(stage.session_id());
    let (stage_session, stage_revision) = stage.stage_identity();
    let identity = ProjectionIdentity {
        activation: stage_info.activation_generation,
        stage_session,
        stage_revision,
        projection: projection.generation(),
        target: target.generation,
    };
    receipt.observe(identity);
    let error = stage_handle
        .as_deref()
        .and_then(|stage| stage.error.as_deref())
        .or_else(|| {
            current_projection
                .then(|| projection.last_error())
                .flatten()
        });
    let can_capture = error.is_none()
        && current_projection
        && projection.readiness() == ProjectionReadiness::Ready
        && projection.native_instance_dependencies_ready()
        && !health
            .as_deref()
            .is_some_and(|health| health.awaiting_payloads);
    let status = if let Some(error) = error {
        InitialGpuProjection::Failed {
            message: error.to_owned(),
        }
    } else if can_capture {
        let started = match *started {
            Some((admitted, at)) if admitted == identity => at,
            _ => {
                let now = Instant::now();
                *started = Some((identity, now));
                now
            }
        };
        let status = receipt.status(identity);
        if status == InitialGpuProjection::Pending && started.elapsed() >= GPU_RECEIPT_TIMEOUT {
            receipt.finish(
                identity,
                InitialGpuProjection::Failed {
                    message: "The renderer did not complete the initial GPU projection.".to_owned(),
                },
            );
            receipt.status(identity)
        } else {
            status
        }
    } else {
        InitialGpuProjection::Pending
    };
    capture.identity = (can_capture && status == InitialGpuProjection::Pending).then_some(identity);
    // Pending is already installed. Latch only the first terminal result.
    if status != InitialGpuProjection::Pending {
        stage_info.initial_gpu_projection = Some(status);
    }
}

fn extract_projection(
    capture: Extract<Res<ProjectionCaptureRequest>>,
    target: Extract<Option<Res<OffscreenTarget>>>,
    cameras: Extract<Query<(Entity, &Camera, &RenderTarget), With<ArcballCamera>>>,
    geometry: Extract<
        Query<
            (
                &Mesh3d,
                Option<&MeshMaterial3d<StandardMaterial>>,
                Option<&WarmPreviewMesh>,
            ),
            Or<(
                With<UsdPrimRef>,
                With<SceneResidencyOccurrence>,
                With<WarmPreviewMesh>,
            )>,
        >,
    >,
    mut input: ResMut<ExtractedProjection>,
) {
    input.identity = None;
    let (Some(identity), Some(target)) = (capture.identity, target.as_deref()) else {
        return;
    };
    input.identity = Some(identity);
    input.output = Some(target.image_handle.id());
    input.preview = capture.preview_camera.is_some();
    input.camera = capture.preview_camera.or_else(|| {
        cameras.iter().find_map(|(entity, camera, render_target)| {
            (camera.is_active && render_target.as_image() == Some(&target.image_handle))
                .then_some(entity)
        })
    });
    input.assets.clear();
    for (mesh, material, preview) in &geometry {
        if input.preview == preview.is_some() {
            input.assets.push(ProjectionAsset {
                mesh: mesh.id(),
                material: material.map(|material| material.id().untyped()),
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn submit_projection_readback(
    input: Res<ExtractedProjection>,
    receipt: Res<ProjectionReadback>,
    preview_receipt: Res<PreviewReadback>,
    views: Query<(
        &ExtractedCamera,
        &ViewTarget,
        &ExtractedView,
        &RenderVisibleEntities,
    )>,
    images: Res<RenderAssets<GpuImage>>,
    meshes: Res<RenderAssets<RenderMesh>>,
    allocations: Res<MeshAllocator>,
    materials: Res<ErasedRenderAssets<PreparedMaterial>>,
    mesh_instances: Res<RenderMeshInstances>,
    material_instances: Res<RenderMaterialInstances>,
    pipelines: (Res<PipelineCache>, Res<SpecializedMaterialPipelineCache>),
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut submitted: Local<Option<ProjectionIdentity>>,
) {
    let (Some(identity), Some(output), Some(owner_camera)) =
        (input.identity, input.output, input.camera)
    else {
        return;
    };
    let receipt = if input.preview {
        &preview_receipt.0
    } else {
        &receipt
    };
    if *submitted == Some(identity)
        || receipt.is_complete(identity)
        || !input.assets.iter().all(|asset| {
            meshes.get(asset.mesh).is_some_and(|mesh| {
                allocations.mesh_vertex_slice(&asset.mesh).is_some()
                    && (!mesh.indexed() || allocations.mesh_index_slice(&asset.mesh).is_some())
            }) && asset
                .material
                .is_none_or(|material| materials.get(material).is_some())
        })
    {
        return;
    }
    let mut output_written = false;
    let mut preview_has_geometry = false;
    for (camera, view_target, view, visible) in &views {
        if !matches!(&camera.target, Some(NormalizedRenderTarget::Image(image)) if image.handle.id() == output)
            || view.retained_view_entity.main_entity.id() != owner_camera
        {
            continue;
        }
        output_written |= view_target.needs_present();
        let Some(visible) = visible.get::<Mesh3d>() else {
            continue;
        };
        let view_pipelines = pipelines.1.get(&view.retained_view_entity);
        let entities = visible
            .entities_cpu_culling
            .iter()
            .map(|(_, entity)| entity)
            .chain(visible.entities_gpu_culling.keys());
        for entity in entities {
            // Overlay and other custom draws without a Bevy material do not
            // belong to the initial PBR projection.
            let Some(material) = material_instances.instances.get(entity) else {
                continue;
            };
            let Some(mesh) = mesh_instances.render_mesh_queue_data(*entity) else {
                return;
            };
            let mesh_id = mesh.mesh_asset_id();
            let Some(render_mesh) = meshes.get(mesh_id) else {
                return;
            };
            if allocations.mesh_vertex_slice(&mesh_id).is_none()
                || (render_mesh.indexed() && allocations.mesh_index_slice(&mesh_id).is_none())
                || materials.get(material.asset_id).is_none()
                || !view_pipelines
                    .and_then(|view| view.get(entity))
                    .is_some_and(|pipeline| pipelines.0.get_render_pipeline(*pipeline).is_some())
            {
                return;
            }
            preview_has_geometry = true;
        }
    }
    if !output_written || (input.preview && !preview_has_geometry) {
        return;
    }
    let Some(image) = images.get(output) else {
        return;
    };
    if image.texture_descriptor.format != TextureFormat::Rgba8UnormSrgb {
        return;
    }

    // The render graph has written to this exact offscreen attachment. A one-texel
    // copy on the same queue gives this activation a small, independent GPU fence.
    // The texel value is immaterial and carries no claim about client presentation.
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("initial projection readback"),
        size: 4,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        image.texture.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &buffer,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: None,
                rows_per_image: None,
            },
        },
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    *submitted = Some(identity);

    let receipt = receipt.clone();
    let mapped = buffer.clone();
    buffer
        .slice(..)
        .map_async(MapMode::Read, move |result| match result {
            Ok(()) => {
                mapped.unmap();
                receipt.complete(identity);
            }
            Err(error) => receipt.finish(
                identity,
                InitialGpuProjection::Failed {
                    message: format!("Initial GPU projection readback failed: {error}"),
                },
            ),
        });
}

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/rendering/offscreen/projection_readback_tests.rs"]
mod tests;
