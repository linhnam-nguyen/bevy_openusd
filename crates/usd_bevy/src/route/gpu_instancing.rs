//! GPU instancing for large point instancers.
//! Adapted from openusd-rs/bevy_openusd f2d474feeb80450597fffd015c787a0469bbbeb1 (MIT).
//!
//! A supported `PointInstancer` above [`UsdGpuInstancing::threshold`]
//! stores its presentation transforms in GPU buffers, split into
//! chunks per prototype, and each prototype mesh draws all of its chunk's
//! instances with one indirect draw. Every frame a compute pass keeps only the
//! instances inside the view and larger than [`UsdGpuInstancing::min_pixels`]
//! on screen, so tens of millions of instances cost only what is visible.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bevy::camera::primitives::{Aabb, MeshAabb};
use bevy::camera::visibility::{self, VisibilityClass};
use bevy::core_pipeline::core_3d::{CORE_3D_DEPTH_FORMAT, Transparent3d, TransparentSortingInfo3d};
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::ecs::{
    query::ROQueryItem,
    system::{
        SystemParam, SystemParamItem,
        lifetimeless::{Read, SRes},
    },
};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    MeshPipeline, MeshPipelineKey, MeshPipelineSystems, PrepassPipeline, SetMeshViewBindGroup,
    SetMeshViewBindingArrayBindGroup, SetPrepassViewBindGroup, SetPrepassViewEmptyBindGroup,
    Shadow, ShadowBatchSetKey, ShadowBinKey, ViewKeyCache, ViewLightEntities,
};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::{
    Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems,
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    mesh::{RenderMesh, RenderMeshBufferInfo, allocator::MeshAllocator},
    render_asset::RenderAssets,
    render_phase::{
        AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, InputUniformIndex, PhaseItem,
        PhaseItemExtraIndex, RenderCommand, RenderCommandResult, SetItemPipeline,
        TrackedRenderPass, ViewBinnedRenderPhases, ViewSortedRenderPhases,
    },
    render_resource::{
        BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer,
        BufferDescriptor, BufferInitDescriptor, BufferUsages, CachedComputePipelineId,
        CompareFunction, ComputePassDescriptor, ComputePipelineDescriptor, DepthStencilState,
        DownlevelFlags, PipelineCache, PrimitiveState, RenderPipelineDescriptor, ShaderStages,
        SpecializedMeshPipeline, SpecializedMeshPipelineError, SpecializedMeshPipelines,
        VertexState, binding_types,
    },
    renderer::{RenderAdapter, RenderContext, RenderDevice, RenderQueue, ViewQuery},
    sync_world::{MainEntity, RenderEntity, SyncToRenderWorld},
    view::{ExtractedView, RenderVisibleEntities, RetainedViewEntity},
};
use bevy::shader::{Shader, ShaderDefVal};
use bevy::shape::ViewFrustum;
use bytemuck::{Pod, Zeroable};

mod picking;
pub(in crate::route) use picking::MAX_TRIANGLE_WORK;
pub use picking::{GpuPointHit, GpuPointPickResult, GpuPointPicker};

/// Instances per chunk: one buffer, one culling dispatch, one bounding box.
pub(crate) const CHUNK: usize = 1 << 20;
const WORKGROUP: u32 = 256;
pub(crate) const PROMOTION_BUDGET: usize = 64;

/// The renderer's projection of authoritative point selection.
#[derive(Resource, Default, PartialEq, Eq)]
pub(crate) struct PointPresentations {
    pub(crate) selected: std::collections::BTreeMap<String, Vec<(i64, u64)>>,
    pub(crate) over_budget: bool,
}
/// Byte stride between per-view culling parameters in their shared buffer.
const VIEW_STRIDE: u64 = 256;

/// Turns on GPU instancing for point instancers. Present in the main world
/// once [`UsdGpuInstancingPlugin`] is added.
#[derive(Resource, ExtractResource, Debug, Clone, Copy)]
#[extract_app(RenderApp)]
pub struct UsdGpuInstancing {
    /// Admit supported PointInstancers to the GPU route.
    pub enabled: bool,
    /// Whether current viewer effects can use the packed renderer.
    pub presentation_compatible: bool,
    /// Instancers with more instances than this are drawn on the GPU; smaller
    /// ones keep one entity per instance.
    pub threshold: usize,
    /// Instances whose bounding sphere projects to a smaller radius than this
    /// many pixels are not drawn.
    pub min_pixels: f32,
    /// Whether the instances a camera draws also cast its directional light
    /// shadows.
    pub shadows: bool,
    /// Upper bound on resident instance and culling buffers per instancer.
    pub instance_buffer_budget: u64,
    /// Upper bound on records in one GPU culling chunk.
    pub max_chunk_instances: usize,
}

impl Default for UsdGpuInstancing {
    fn default() -> Self {
        Self {
            enabled: false,
            presentation_compatible: true,
            threshold: 4096,
            min_pixels: 0.75,
            shadows: false,
            instance_buffer_budget: 256 * 1024 * 1024,
            max_chunk_instances: CHUNK,
        }
    }
}

/// Draws large point instancers with GPU culling and indirect draws.
pub struct UsdGpuInstancingPlugin;

/// Scoped to presentation reprojection that only promotes or demotes selected rows.
#[derive(Resource)]
pub(crate) struct SelectionOnlyPointProjection;

#[derive(Resource)]
struct InstancingShaderLibraries {
    _handles: [Handle<Shader>; 2],
}

impl Plugin for UsdGpuInstancingPlugin {
    fn build(&self, app: &mut App) {
        if app.get_sub_app(RenderApp).is_none() {
            return;
        }
        bevy::asset::embedded_asset!(app, "gpu_instancing.wesl");
        bevy::asset::embedded_asset!(app, "gpu_instancing_cull.wgsl");
        bevy::asset::embedded_asset!(app, "gpu_instancing_shadow.wesl");
        bevy::asset::embedded_asset!(app, "gpu_instancing_types.wesl");
        bevy::asset::embedded_asset!(app, "gpu_instancing_pick.wgsl");
        bevy::asset::embedded_asset!(app, "sheen_functions.wesl");
        let assets = app.world().resource::<AssetServer>();
        let libraries = InstancingShaderLibraries {
            _handles: [
                assets.load("embedded://usd_bevy/route/gpu_instancing_types.wesl"),
                assets.load("embedded://usd_bevy/route/sheen_functions.wesl"),
            ],
        };
        app.insert_resource(libraries);
        app.init_resource::<PointPresentations>();
        if !app.world().contains_resource::<UsdGpuInstancing>() {
            app.init_resource::<UsdGpuInstancing>();
        }
        app.add_plugins(ExtractResourcePlugin::<UsdGpuInstancing>::default());
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render
            .init_resource::<SpecializedMeshPipelines<InstancingPipeline>>()
            .init_resource::<SpecializedMeshPipelines<InstancedShadowPipeline>>()
            .init_resource::<GpuChunks>()
            .init_resource::<CullViews>()
            .init_resource::<QueuedShadows>()
            .add_render_command::<Transparent3d, DrawInstanced>()
            .add_render_command::<Shadow, DrawInstancedShadow>()
            .add_systems(
                RenderStartup,
                (
                    init_pipeline.after(MeshPipelineSystems),
                    init_shadow_pipeline
                        .after(init_pipeline)
                        .after(bevy::pbr::init_prepass_pipeline),
                ),
            )
            .add_systems(ExtractSchedule, extract_draws)
            .add_systems(
                Render,
                (
                    (prepare_chunks, prepare_views).in_set(RenderSystems::PrepareResources),
                    prepare_bind_groups.in_set(RenderSystems::PrepareBindGroups),
                    (queue_draws, queue_shadows).in_set(RenderSystems::QueueMeshes),
                ),
            )
            // Shadows draw the instances the camera kept, so culling runs first.
            .add_systems(
                Core3d,
                cull_instances
                    .before(bevy::pbr::per_view_shadow_pass::<{ bevy::pbr::EARLY_SHADOW_PASS }>)
                    .before(Core3dSystems::MainPass),
            );
        picking::setup(app);
    }
}

/// One instance in its instancer's space: translation, rotation as four
/// snorm16 values, and scale. Matches `Instance` in the WGSL.
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
#[repr(C)]
pub(crate) struct GpuInstance {
    translation: [f32; 3],
    rotation: [u32; 2],
    scale: [f32; 3],
}

const _: () = assert!(size_of::<GpuInstance>() == 32);

#[derive(Clone, Copy)]
pub(crate) struct PackedRow {
    pub(crate) logical_id: i64,
    pub(crate) transform: GpuInstance,
    pub(crate) enabled: u32,
}

impl GpuInstance {
    pub(crate) fn new(translation: Vec3, rotation: Quat, scale: Vec3) -> Self {
        let pack = |a: f32, b: f32| {
            let snorm = |v: f32| u32::from((v.clamp(-1.0, 1.0) * 32767.0).round() as i16 as u16);
            snorm(a) | (snorm(b) << 16)
        };
        let q = rotation.normalize();
        Self {
            translation: translation.to_array(),
            rotation: [pack(q.x, q.y), pack(q.z, q.w)],
            scale: scale.to_array(),
        }
    }

    /// Where a prototype-space sphere lands in instancer space.
    fn sphere(&self, sphere: Vec4) -> (Vec3, f32) {
        let unpack = |v: u32| {
            let snorm = |b: u32| (f32::from(b as u16 as i16) / 32767.0).max(-1.0);
            (snorm(v & 0xffff), snorm(v >> 16))
        };
        let (x, y) = unpack(self.rotation[0]);
        let (z, w) = unpack(self.rotation[1]);
        let rotation = Quat::from_xyzw(x, y, z, w).normalize();
        let scale = Vec3::from_array(self.scale);
        let center = rotation * (sphere.truncate() * scale) + Vec3::from_array(self.translation);
        (center, sphere.w * scale.abs().max_element())
    }
}

/// Instances of one prototype, uploaded to the GPU once and shared by every
/// mesh of that prototype.
pub(crate) struct InstanceChunk {
    id: u64,
    instances: Vec<GpuInstance>,
    logical_ids: Vec<i64>,
    enabled: Vec<u32>,
    /// The prototype's bounding sphere in prototype space: center and radius.
    sphere: Vec4,
}

impl InstanceChunk {
    /// Wraps `instances` as a chunk, with its bounding box in instancer space.
    fn new(rows: &[PackedRow], sphere: Vec4, previous: Option<&Arc<Self>>) -> (Arc<Self>, Aabb) {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let logical_ids: Vec<_> = rows.iter().map(|row| row.logical_id).collect();
        let instances: Vec<_> = rows.iter().map(|row| row.transform).collect();
        let enabled: Vec<_> = rows.iter().map(|row| row.enabled).collect();
        let (mut min, mut max) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
        for (instance, active) in instances.iter().zip(&enabled) {
            if *active == 0 {
                continue;
            }
            let (center, radius) = instance.sphere(sphere);
            min = min.min(center - radius);
            max = max.max(center + radius);
        }
        let bounds = if min.is_finite() {
            Aabb::from_min_max(min, max)
        } else {
            Aabb::from_min_max(Vec3::ZERO, Vec3::ZERO)
        };
        let compatible = previous.filter(|previous| previous.logical_ids == logical_ids);
        if let Some(previous) = compatible
            && previous.instances == instances
            && previous.enabled == enabled
            && previous.sphere == sphere
        {
            return (previous.clone(), bounds);
        }
        let chunk = Self {
            id: compatible.map_or_else(|| NEXT.fetch_add(1, Ordering::Relaxed), |old| old.id),
            instances,
            logical_ids,
            enabled,
            sphere,
        };
        (Arc::new(chunk), bounds)
    }
}

/// The material inputs an instanced draw shades with.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InstancedMaterial {
    base_color: LinearRgba,
    emissive: LinearRgba,
    roughness: f32,
    metallic: f32,
    /// Disney sheen's weight and tint.
    sheen: [f32; 2],
}

impl From<&StandardMaterial> for InstancedMaterial {
    fn from(material: &StandardMaterial) -> Self {
        Self {
            base_color: material.base_color.to_linear(),
            emissive: material.emissive,
            roughness: material.perceptual_roughness,
            metallic: material.metallic,
            sheen: [0.0; 2],
        }
    }
}

/// One prototype mesh drawn for every instance of a chunk.
#[derive(Component, Clone)]
#[require(VisibilityClass, Transform, Visibility, SyncToRenderWorld)]
#[component(on_add = visibility::add_visibility_class::<UsdInstancedDraw>)]
pub struct UsdInstancedDraw {
    pub(crate) mesh: Handle<Mesh>,
    /// The mesh's placement inside the prototype.
    pub(crate) part: Mat4,
    pub(crate) material: InstancedMaterial,
    pub(crate) chunk: Arc<InstanceChunk>,
    instancer: Arc<str>,
    pick_mesh: Arc<picking::PickMesh>,
    key: DrawKey,
}

impl UsdInstancedDraw {
    /// How many instances this draw covers before culling.
    pub fn instance_count(&self) -> usize {
        self.chunk.instances.len()
    }

    /// Stable USD ID for a source slot. Compacted indices are view-local.
    pub fn logical_id(&self, source_slot: usize) -> Option<i64> {
        self.chunk.logical_ids.get(source_slot).copied()
    }
}

#[derive(Component)]
struct ExtractedDraw {
    mesh: AssetId<Mesh>,
    world: Mat4,
    part: Mat4,
    material: InstancedMaterial,
    chunk: Arc<InstanceChunk>,
    instancer: Arc<str>,
    pick_mesh: Arc<picking::PickMesh>,
}

fn extract_draws(
    mut commands: Commands,
    draws: Extract<Query<(RenderEntity, &UsdInstancedDraw, &GlobalTransform)>>,
) {
    for (entity, draw, transform) in &draws {
        commands.entity(entity).insert(ExtractedDraw {
            mesh: draw.mesh.id(),
            world: transform.to_matrix(),
            part: draw.part,
            material: draw.material,
            chunk: draw.chunk.clone(),
            instancer: draw.instancer.clone(),
            pick_mesh: draw.pick_mesh.clone(),
        });
    }
}

/// Per-chunk culling inputs. Matches `Chunk` in the culling WGSL.
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct ChunkUniform {
    world_from_instancer: [[f32; 4]; 4],
    sphere: [f32; 4],
    len: u32,
    world_scale: f32,
    pad: [f32; 2],
}

/// Per-view culling inputs. Matches `CullView` in the culling WGSL.
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct CullView {
    planes: [[f32; 4]; 6],
    /// Camera position, and pixels per unit of radius at unit distance.
    camera: [f32; 4],
    min_pixels: f32,
    pad: [f32; 3],
}

/// Per-draw shading inputs. Matches `Draw` in the drawing WGSL.
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct DrawUniform {
    world_from_instancer: [[f32; 4]; 4],
    part: [[f32; 4]; 4],
    base_color: [f32; 4],
    emissive: [f32; 4],
    roughness: f32,
    metallic: f32,
    sheen: f32,
    sheen_tint: f32,
}

struct GpuChunk {
    instances: Buffer,
    enabled: Buffer,
    uploaded: Arc<InstanceChunk>,
    visible: Buffer,
    count: Buffer,
    uniform: Buffer,
    len: u32,
    cull: Option<BindGroup>,
    seen: u64,
}

#[derive(Resource, Default)]
struct GpuChunks {
    chunks: HashMap<u64, GpuChunk>,
    frame: u64,
    /// The view buffer generation the culling bind groups were built for.
    views: u64,
}

#[derive(Resource, Default)]
struct CullViews {
    buffer: Option<Buffer>,
    capacity: u64,
    generation: u64,
}

/// Where a view's culling parameters sit in [`CullViews`].
#[derive(Component, Clone, Copy)]
struct CullViewOffset(u32);

#[derive(Component)]
struct GpuDraw {
    args: Buffer,
    uniform: Buffer,
    bind_group: BindGroup,
    chunk: u64,
}

#[derive(Resource)]
struct InstancingPipeline {
    mesh_pipeline: MeshPipeline,
    shader: Handle<Shader>,
    draw_layout: BindGroupLayoutDescriptor,
    cull_layout: BindGroupLayoutDescriptor,
    cull: CachedComputePipelineId,
}

fn init_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mesh_pipeline: Res<MeshPipeline>,
    pipeline_cache: Res<PipelineCache>,
) {
    let draw_layout = BindGroupLayoutDescriptor::new(
        "usd_instanced_draw",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (
                binding_types::storage_buffer_read_only_sized(false, None),
                binding_types::storage_buffer_read_only_sized(false, None),
                binding_types::uniform_buffer_sized(false, None),
            ),
        ),
    );
    let cull_layout = BindGroupLayoutDescriptor::new(
        "usd_instanced_cull",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                binding_types::storage_buffer_read_only_sized(false, None),
                binding_types::storage_buffer_sized(false, None),
                binding_types::storage_buffer_sized(false, None),
                binding_types::uniform_buffer_sized(false, None),
                binding_types::uniform_buffer_sized(true, None),
                binding_types::storage_buffer_read_only_sized(false, None),
            ),
        ),
    );
    let cull = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("usd_instanced_cull".into()),
        layout: vec![cull_layout.clone()],
        shader: asset_server.load("embedded://usd_bevy/route/gpu_instancing_cull.wgsl"),
        ..default()
    });
    commands.insert_resource(InstancingPipeline {
        mesh_pipeline: mesh_pipeline.clone(),
        shader: asset_server.load("embedded://usd_bevy/route/gpu_instancing.wesl"),
        draw_layout,
        cull_layout,
        cull,
    });
}

impl SpecializedMeshPipeline for InstancingPipeline {
    type Key = MeshPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh_pipeline.specialize(key, layout)?;
        descriptor.label = Some("usd_instanced".into());
        descriptor.layout.truncate(2);
        descriptor.layout.push(self.draw_layout.clone());
        descriptor.primitive.cull_mode = Some(bevy::render::render_resource::Face::Back);
        descriptor.vertex.shader = self.shader.clone();
        let material_group = ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3);
        descriptor.vertex.shader_defs.push(material_group.clone());
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = self.shader.clone();
            fragment.shader_defs.push(material_group);
        }
        Ok(descriptor)
    }
}

/// Depth-only drawing of instanced prototypes into directional shadow maps.
#[derive(Resource)]
struct InstancedShadowPipeline {
    view_layout: BindGroupLayoutDescriptor,
    empty_layout: BindGroupLayoutDescriptor,
    draw_layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
    unclipped_depth: bool,
}

fn init_shadow_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    instancing: Option<Res<InstancingPipeline>>,
    prepass: Option<Res<PrepassPipeline>>,
) {
    let (Some(instancing), Some(prepass)) = (instancing, prepass) else {
        return;
    };
    commands.insert_resource(InstancedShadowPipeline {
        view_layout: prepass.view_layout_no_motion_vectors.clone(),
        empty_layout: prepass.empty_layout.clone(),
        draw_layout: instancing.draw_layout.clone(),
        shader: asset_server.load("embedded://usd_bevy/route/gpu_instancing_shadow.wesl"),
        unclipped_depth: prepass.depth_clip_control_supported,
    });
}

impl SpecializedMeshPipeline for InstancedShadowPipeline {
    type Key = MeshPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let shader_defs = if self.unclipped_depth {
            Vec::new()
        } else {
            vec!["CLAMP_DEPTH".into()]
        };
        Ok(RenderPipelineDescriptor {
            label: Some("usd_instanced_shadow".into()),
            layout: vec![
                self.view_layout.clone(),
                self.empty_layout.clone(),
                self.draw_layout.clone(),
            ],
            vertex: VertexState {
                shader: self.shader.clone(),
                shader_defs,
                buffers: vec![
                    layout
                        .0
                        .get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?,
                ],
                ..default()
            },
            primitive: PrimitiveState {
                topology: key.primitive_topology(),
                strip_index_format: key.strip_index_format(),
                unclipped_depth: self.unclipped_depth,
                ..default()
            },
            depth_stencil: Some(DepthStencilState {
                format: CORE_3D_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: default(),
                bias: default(),
            }),
            ..default()
        })
    }
}

fn prepare_chunks(
    draws: Query<&ExtractedDraw>,
    mut chunks: ResMut<GpuChunks>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    chunks.frame += 1;
    let frame = chunks.frame;
    for draw in &draws {
        let source = &draw.chunk;
        if chunks
            .chunks
            .get(&source.id)
            .is_some_and(|chunk| chunk.seen == frame)
        {
            continue;
        }
        let chunk = chunks.chunks.entry(source.id).or_insert_with(|| {
            let len = source.instances.len() as u32;
            bevy::log::debug!(target: "usd_bevy::route::gpu_instancing",
                chunk = source.id, rows = len, record_bytes = u64::from(len) * 40,
                "allocated packed instance buffers");
            let storage = |label, size: u64, usage| {
                device.create_buffer(&BufferDescriptor {
                    label: Some(label),
                    size: size.max(4),
                    usage,
                    mapped_at_creation: false,
                })
            };
            GpuChunk {
                instances: device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("usd_instances"),
                    contents: bytemuck::cast_slice(&source.instances),
                    usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
                }),
                enabled: device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("usd_instance_masks"),
                    contents: bytemuck::cast_slice(&source.enabled),
                    usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                }),
                uploaded: source.clone(),
                visible: storage(
                    "usd_visible_instances",
                    u64::from(len) * 4,
                    BufferUsages::STORAGE | BufferUsages::COPY_SRC,
                ),
                count: storage(
                    "usd_visible_count",
                    4,
                    BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
                ),
                uniform: storage(
                    "usd_instance_chunk",
                    size_of::<ChunkUniform>() as u64,
                    BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                ),
                len,
                cull: None,
                seen: frame,
            }
        });
        if !Arc::ptr_eq(&chunk.uploaded, source) {
            let transform_bytes = upload_dirty(
                &queue,
                &chunk.instances,
                &chunk.uploaded.instances,
                &source.instances,
            );
            let mask_bytes = upload_dirty(
                &queue,
                &chunk.enabled,
                &chunk.uploaded.enabled,
                &source.enabled,
            );
            if transform_bytes > 0 || mask_bytes > 0 {
                bevy::log::debug!(target: "usd_bevy::route::gpu_instancing",
                    chunk = source.id, transform_bytes, mask_bytes, "updated packed instance ranges");
            }
            chunk.uploaded = source.clone();
        }
        chunk.seen = frame;
        let x = draw.world.x_axis.truncate().abs();
        let y = draw.world.y_axis.truncate().abs();
        let z = draw.world.z_axis.truncate().abs();
        let norm_one = x.element_sum().max(y.element_sum()).max(z.element_sum());
        let norm_infinity = (x + y + z).max_element();
        let uniform = ChunkUniform {
            world_from_instancer: draw.world.to_cols_array_2d(),
            sphere: source.sphere.to_array(),
            len: chunk.len,
            world_scale: (norm_one * norm_infinity).sqrt(),
            pad: [0.0; 2],
        };
        queue.write_buffer(&chunk.uniform, 0, bytemuck::bytes_of(&uniform));
    }
    chunks.chunks.retain(|_, chunk| chunk.seen + 2 >= frame);
}

fn upload_dirty<T: Pod + PartialEq>(
    queue: &RenderQueue,
    buffer: &Buffer,
    old: &[T],
    new: &[T],
) -> usize {
    debug_assert_eq!(old.len(), new.len());
    let mut start = None;
    let mut bytes = 0;
    for index in 0..=new.len() {
        if index < new.len() && old[index] != new[index] {
            start.get_or_insert(index);
        } else if let Some(start) = start.take() {
            let data = bytemuck::cast_slice(&new[start..index]);
            queue.write_buffer(buffer, (start * size_of::<T>()) as u64, data);
            bytes += data.len();
        }
    }
    bytes
}

fn prepare_views(
    mut commands: Commands,
    views: Query<(Entity, &ExtractedView)>,
    mut cull_views: ResMut<CullViews>,
    settings: Option<Res<UsdGpuInstancing>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let min_pixels = settings.map_or(UsdGpuInstancing::default().min_pixels, |s| s.min_pixels);
    let mut bytes = Vec::new();
    for (index, (entity, view)) in views.iter().enumerate() {
        let clip_from_world = view
            .clip_from_world
            .unwrap_or_else(|| view.clip_from_view * view.world_from_view.to_matrix().inverse());
        let frustum = ViewFrustum::from_clip_from_world(&clip_from_world);
        let mut planes = [[0.0, 0.0, 0.0, 1.0]; 6];
        for (plane, half_space) in planes.iter_mut().zip(&frustum.half_spaces[..5]) {
            *plane = half_space.normal_d().to_array();
        }
        let orthographic = view.clip_from_view.w_axis.w == 1.0;
        let pixels = view.clip_from_view.y_axis.y * view.viewport.w as f32 * 0.5;
        let camera = view.world_from_view.translation();
        let cull = CullView {
            planes,
            camera: camera.extend(pixels).to_array(),
            min_pixels: if orthographic { 0.0 } else { min_pixels },
            pad: [0.0; 3],
        };
        bytes.resize(index * VIEW_STRIDE as usize, 0);
        bytes.extend_from_slice(bytemuck::bytes_of(&cull));
        commands
            .entity(entity)
            .insert(CullViewOffset((index as u64 * VIEW_STRIDE) as u32));
    }
    if bytes.is_empty() {
        return;
    }
    let needed = bytes.len() as u64;
    if cull_views.capacity < needed {
        cull_views.capacity = needed.next_power_of_two().max(VIEW_STRIDE * 4);
        cull_views.buffer = Some(device.create_buffer(&BufferDescriptor {
            label: Some("usd_instanced_views"),
            size: cull_views.capacity,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        cull_views.generation += 1;
    }
    queue.write_buffer(cull_views.buffer.as_ref().unwrap(), 0, &bytes);
}

#[derive(SystemParam)]
struct InstancingBindResources<'w> {
    pipeline_cache: Res<'w, PipelineCache>,
    meshes: Res<'w, RenderAssets<RenderMesh>>,
    allocator: Res<'w, MeshAllocator>,
    device: Res<'w, RenderDevice>,
    queue: Res<'w, RenderQueue>,
}

fn prepare_bind_groups(
    mut commands: Commands,
    draws: Query<(Entity, &ExtractedDraw, Option<&GpuDraw>)>,
    mut chunks: ResMut<GpuChunks>,
    cull_views: Res<CullViews>,
    pipeline: Option<Res<InstancingPipeline>>,
    resources: InstancingBindResources,
) {
    let InstancingBindResources {
        pipeline_cache,
        meshes,
        allocator,
        device,
        queue,
    } = resources;
    let (Some(pipeline), Some(views)) = (pipeline, cull_views.buffer.as_ref()) else {
        return;
    };
    let rebuild = chunks.views != cull_views.generation;
    chunks.views = cull_views.generation;
    let cull_layout = pipeline_cache.get_bind_group_layout(&pipeline.cull_layout);
    for chunk in chunks.chunks.values_mut() {
        if chunk.cull.is_none() || rebuild {
            chunk.cull = Some(device.create_bind_group(
                "usd_instanced_cull",
                &cull_layout,
                &BindGroupEntries::sequential((
                    chunk.instances.as_entire_binding(),
                    chunk.visible.as_entire_binding(),
                    chunk.count.as_entire_binding(),
                    chunk.uniform.as_entire_binding(),
                    bevy::render::render_resource::BufferBinding {
                        buffer: views,
                        offset: 0,
                        size: std::num::NonZero::new(size_of::<CullView>() as u64),
                    },
                    chunk.enabled.as_entire_binding(),
                )),
            ));
        }
    }
    let draw_layout = pipeline_cache.get_bind_group_layout(&pipeline.draw_layout);
    for (entity, draw, existing) in &draws {
        let Some(chunk) = chunks.chunks.get(&draw.chunk.id) else {
            continue;
        };
        let (Some(mesh), Some(vertices), Some(indices)) = (
            meshes.get(draw.mesh),
            allocator.mesh_vertex_slice(&draw.mesh),
            allocator.mesh_index_slice(&draw.mesh),
        ) else {
            continue;
        };
        let RenderMeshBufferInfo::Indexed { count, .. } = mesh.buffer_info else {
            continue;
        };
        let gpu = match existing {
            Some(gpu) if gpu.chunk == draw.chunk.id => None,
            _ => {
                let args = device.create_buffer(&BufferDescriptor {
                    label: Some("usd_instanced_args"),
                    size: 20,
                    usage: BufferUsages::INDIRECT | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let uniform = device.create_buffer(&BufferDescriptor {
                    label: Some("usd_instanced_draw"),
                    size: size_of::<DrawUniform>() as u64,
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bind_group = device.create_bind_group(
                    "usd_instanced_draw",
                    &draw_layout,
                    &BindGroupEntries::sequential((
                        chunk.instances.as_entire_binding(),
                        chunk.visible.as_entire_binding(),
                        uniform.as_entire_binding(),
                    )),
                );
                Some(GpuDraw {
                    args,
                    uniform,
                    bind_group,
                    chunk: draw.chunk.id,
                })
            }
        };
        let target = gpu.as_ref().or(existing).unwrap();
        let args: [u32; 5] = [count, 0, indices.range.start, vertices.range.start, 0];
        queue.write_buffer(&target.args, 0, bytemuck::cast_slice(&args));
        let uniform = DrawUniform {
            world_from_instancer: draw.world.to_cols_array_2d(),
            part: draw.part.to_cols_array_2d(),
            base_color: draw.material.base_color.to_f32_array(),
            emissive: draw.material.emissive.to_f32_array(),
            roughness: draw.material.roughness,
            metallic: draw.material.metallic,
            sheen: draw.material.sheen[0],
            sheen_tint: draw.material.sheen[1],
        };
        queue.write_buffer(&target.uniform, 0, bytemuck::bytes_of(&uniform));
        if let Some(gpu) = gpu {
            commands.entity(entity).insert(gpu);
        }
    }
}

#[derive(SystemParam)]
struct InstancingDrawResources<'w> {
    draw_functions: Res<'w, DrawFunctions<Transparent3d>>,
    pipeline: Option<Res<'w, InstancingPipeline>>,
    pipelines: ResMut<'w, SpecializedMeshPipelines<InstancingPipeline>>,
    pipeline_cache: Res<'w, PipelineCache>,
    meshes: Res<'w, RenderAssets<RenderMesh>>,
}

fn queue_draws(
    resources: InstancingDrawResources,
    // QueueMeshes precedes PrepareBindGroups, which installs GpuDraw.
    draws: Query<&ExtractedDraw>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    views: Query<(&ExtractedView, &RenderVisibleEntities)>,
    view_keys: Res<ViewKeyCache>,
) {
    let InstancingDrawResources {
        draw_functions,
        pipeline,
        mut pipelines,
        pipeline_cache,
        meshes,
    } = resources;
    let Some(pipeline) = pipeline else {
        return;
    };
    let draw_function = draw_functions.read().id::<DrawInstanced>();
    for (view, visible) in &views {
        let (Some(phase), Some(&view_key), Some(visible)) = (
            phases.get_mut(&view.retained_view_entity),
            view_keys.get(&view.retained_view_entity),
            visible.get::<UsdInstancedDraw>(),
        ) else {
            continue;
        };
        for (&entity, &main_entity) in visible.iter_visible() {
            let Ok(draw) = draws.get(entity) else {
                continue;
            };
            let Some(mesh) = meshes.get(draw.mesh) else {
                continue;
            };
            let key = view_key
                | MeshPipelineKey::from_primitive_topology_and_strip_index(
                    mesh.primitive_topology(),
                    mesh.index_format(),
                );
            let Ok(pipeline_id) =
                pipelines.specialize(&pipeline_cache, &pipeline, key, &mesh.layout)
            else {
                continue;
            };
            phase.add_transient(Transparent3d {
                sorting_info: TransparentSortingInfo3d::Sorted {
                    mesh_center: draw.world.transform_point3(draw.chunk.sphere.truncate()),
                    depth_bias: 0.0,
                },
                entity: (entity, main_entity),
                pipeline: pipeline_id,
                draw_function,
                distance: 0.0,
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::None,
                indexed: true,
            });
        }
    }
}

/// The instanced draws queued last frame into each shadow phase, which keeps
/// its items until they are removed.
#[derive(Resource, Default)]
struct QueuedShadows(HashMap<RetainedViewEntity, Vec<MainEntity>>);

#[derive(SystemParam)]
struct InstancingShadowResources<'w> {
    draw_functions: Res<'w, DrawFunctions<Shadow>>,
    pipeline: Option<Res<'w, InstancedShadowPipeline>>,
    pipelines: ResMut<'w, SpecializedMeshPipelines<InstancedShadowPipeline>>,
    pipeline_cache: Res<'w, PipelineCache>,
    meshes: Res<'w, RenderAssets<RenderMesh>>,
    allocator: Res<'w, MeshAllocator>,
}

/// Queues the draws each camera sees into its directional shadow cascades.
fn queue_shadows(
    resources: InstancingShadowResources,
    draws: Query<&ExtractedDraw>,
    mut phases: ResMut<ViewBinnedRenderPhases<Shadow>>,
    cameras: Query<(&RenderVisibleEntities, &ViewLightEntities)>,
    light_views: Query<&ExtractedView>,
    mut queued: ResMut<QueuedShadows>,
    settings: Option<Res<UsdGpuInstancing>>,
) {
    let InstancingShadowResources {
        draw_functions,
        pipeline,
        mut pipelines,
        pipeline_cache,
        meshes,
        allocator,
    } = resources;
    for (view, entities) in queued.0.drain() {
        if let Some(phase) = phases.get_mut(&view) {
            for entity in entities {
                phase.remove(entity);
            }
        }
    }
    let (Some(pipeline), true) = (pipeline, settings.is_some_and(|settings| settings.shadows))
    else {
        return;
    };
    let draw_function = draw_functions.read().id::<DrawInstancedShadow>();
    for (visible, lights) in &cameras {
        let Some(visible) = visible.get::<UsdInstancedDraw>() else {
            continue;
        };
        for light in &lights.lights {
            let Ok(light_view) = light_views.get(*light) else {
                continue;
            };
            let Some(phase) = phases.get_mut(&light_view.retained_view_entity) else {
                continue;
            };
            let added = queued.0.entry(light_view.retained_view_entity).or_default();
            for (&entity, &main_entity) in visible.iter_visible() {
                let Ok(draw) = draws.get(entity) else {
                    continue;
                };
                let (Some(mesh), Some(slabs)) =
                    (meshes.get(draw.mesh), allocator.mesh_slabs(&draw.mesh))
                else {
                    continue;
                };
                let key = MeshPipelineKey::from_primitive_topology_and_strip_index(
                    mesh.primitive_topology(),
                    mesh.index_format(),
                );
                let Ok(pipeline_id) =
                    pipelines.specialize(&pipeline_cache, &pipeline, key, &mesh.layout)
                else {
                    continue;
                };
                phase.add(
                    ShadowBatchSetKey {
                        pipeline: pipeline_id,
                        draw_function,
                        material_bind_group_index: None,
                        slabs,
                    },
                    ShadowBinKey {
                        asset_id: draw.mesh.untyped(),
                    },
                    (entity, main_entity),
                    InputUniformIndex::default(),
                    BinnedRenderPhaseType::NonMesh,
                );
                added.push(main_entity);
            }
        }
    }
}

fn cull_instances(
    view: ViewQuery<(&CullViewOffset, &RenderVisibleEntities)>,
    draws: Query<(&ExtractedDraw, &GpuDraw)>,
    chunks: Res<GpuChunks>,
    pipeline: Option<Res<InstancingPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (offset, visible) = view.into_inner();
    let Some(visible) = visible.get::<UsdInstancedDraw>() else {
        return;
    };
    let Some(cull) = pipeline
        .as_ref()
        .and_then(|pipeline| pipeline_cache.get_compute_pipeline(pipeline.cull))
    else {
        return;
    };
    let mut culled = HashSet::new();
    let mut copies = Vec::new();
    for (&entity, _) in visible.iter_visible() {
        let Ok((draw, gpu)) = draws.get(entity) else {
            continue;
        };
        let Some(chunk) = chunks.chunks.get(&draw.chunk.id) else {
            continue;
        };
        culled.insert(draw.chunk.id);
        copies.push((chunk, gpu));
    }
    if copies.is_empty() {
        return;
    }
    let encoder = ctx.command_encoder();
    for id in &culled {
        encoder.clear_buffer(&chunks.chunks[id].count, 0, None);
    }
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("usd_instanced_cull"),
            timestamp_writes: None,
        });
        pass.set_pipeline(cull);
        for id in &culled {
            let chunk = &chunks.chunks[id];
            let Some(bind_group) = &chunk.cull else {
                continue;
            };
            pass.set_bind_group(0, bind_group, &[offset.0]);
            pass.dispatch_workgroups(chunk.len.div_ceil(WORKGROUP), 1, 1);
        }
    }
    for (chunk, gpu) in copies {
        encoder.copy_buffer_to_buffer(&chunk.count, 0, &gpu.args, 4, 4);
    }
}

type DrawInstanced = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    DrawInstancedIndirect,
);

type DrawInstancedShadow = (
    SetItemPipeline,
    SetPrepassViewBindGroup<0>,
    SetPrepassViewEmptyBindGroup<1>,
    DrawInstancedIndirect,
);

struct DrawInstancedIndirect;

impl<P: PhaseItem> RenderCommand<P> for DrawInstancedIndirect {
    type Param = (SRes<RenderAssets<RenderMesh>>, SRes<MeshAllocator>);
    type ViewQuery = ();
    type ItemQuery = (Read<ExtractedDraw>, Read<GpuDraw>);

    fn render<'w>(
        _: &P,
        _: ROQueryItem<'w, '_, Self::ViewQuery>,
        item: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        (meshes, allocator): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some((draw, gpu)) = item else {
            return RenderCommandResult::Skip;
        };
        let allocator = allocator.into_inner();
        let Some(mesh) = meshes.into_inner().get(draw.mesh) else {
            return RenderCommandResult::Skip;
        };
        let (Some(vertices), Some(indices)) = (
            allocator.mesh_vertex_slice(&draw.mesh),
            allocator.mesh_index_slice(&draw.mesh),
        ) else {
            return RenderCommandResult::Skip;
        };
        let RenderMeshBufferInfo::Indexed { index_format, .. } = mesh.buffer_info else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(2, &gpu.bind_group, &[]);
        pass.set_vertex_buffer(0, vertices.buffer.slice(..));
        pass.set_index_buffer(indices.buffer.slice(..), index_format);
        pass.draw_indexed_indirect(&gpu.args, 0);
        RenderCommandResult::Success
    }
}

/// Whether a point instancer with `count` instances draws on the GPU.
pub(crate) fn chunk_capacity(world: &World, count: usize) -> Option<usize> {
    let settings = world.get_resource::<UsdGpuInstancing>()?;
    let packed_bytes = u64::try_from(count)
        .ok()?
        .checked_mul((size_of::<GpuInstance>() + 2 * size_of::<u32>()) as u64)?;
    if !settings.enabled
        || !settings.presentation_compatible
        || count <= settings.threshold
        || packed_bytes > settings.instance_buffer_budget
    {
        return None;
    }
    let device = world.get_resource::<RenderDevice>()?;
    let adapter = world.get_resource::<RenderAdapter>()?;
    let flags = adapter.get_downlevel_capabilities().flags;
    if !flags.contains(DownlevelFlags::COMPUTE_SHADERS | DownlevelFlags::INDIRECT_EXECUTION) {
        return None;
    }
    let limits = device.limits();
    if limits.max_storage_buffers_per_shader_stage < 5
        || limits.max_compute_invocations_per_workgroup < WORKGROUP
        || limits.max_compute_workgroup_size_x < WORKGROUP
    {
        return None;
    }
    let max_bytes = limits
        .max_buffer_size
        .min(u64::from(limits.max_storage_buffer_binding_size));
    let storage = (max_bytes / size_of::<GpuInstance>() as u64) as usize;
    let compute = limits.max_compute_workgroups_per_dimension as usize * WORKGROUP as usize;
    let capacity = CHUNK
        .min(settings.max_chunk_instances)
        .min(storage)
        .min(compute);
    (capacity > 0).then_some(capacity)
}

/// Despawns the instanced draws under `instancer`.
pub(crate) fn clear(world: &mut World, instancer: Entity) {
    let draws: Vec<Entity> = world
        .get::<Children>(instancer)
        .into_iter()
        .flat_map(|children| children.iter())
        .filter(|child| world.get::<UsdInstancedDraw>(*child).is_some())
        .collect();
    for draw in draws {
        world.despawn(draw);
    }
}

pub(crate) fn has_draws(world: &World, instancer: Entity) -> bool {
    world.get::<Children>(instancer).is_some_and(|children| {
        children
            .iter()
            .any(|child| world.get::<UsdInstancedDraw>(child).is_some())
    })
}

/// Spawns one instanced draw per chunk for each prototype mesh.
pub(in crate::route) struct PreparedGpuMesh {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    part: Mat4,
    pick_mesh: Arc<picking::PickMesh>,
}

impl PreparedGpuMesh {
    pub(in crate::route) fn new(
        mesh: Handle<Mesh>,
        source: &Mesh,
        material: Handle<StandardMaterial>,
        part: Mat4,
    ) -> Option<Self> {
        let pick_mesh = Arc::new(picking::PickMesh::read(source)?);
        Some(Self {
            mesh,
            material,
            part,
            pick_mesh,
        })
    }

    pub(in crate::route) fn triangles(&self) -> usize {
        self.pick_mesh.triangles()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DrawKey {
    prototype: usize,
    chunk: usize,
    part: usize,
}

pub(crate) fn patch_transforms(
    world: &mut World,
    instancer: Entity,
    changes: &HashMap<i64, Transform>,
) {
    if changes.is_empty() {
        return;
    }
    let mut updates = HashMap::<u64, Option<(Arc<InstanceChunk>, Aabb)>>::default();
    for entity in existing_draws(world, instancer).into_values() {
        let mut draw = world
            .get::<UsdInstancedDraw>(entity)
            .expect("tracked point draw")
            .clone();
        let updated = updates.entry(draw.chunk.id).or_insert_with(|| {
            let old = &draw.chunk;
            if !old.logical_ids.iter().any(|id| changes.contains_key(id)) {
                return None;
            }
            let rows: Vec<_> = old
                .logical_ids
                .iter()
                .zip(&old.instances)
                .zip(&old.enabled)
                .map(|((&logical_id, previous), &enabled)| PackedRow {
                    logical_id,
                    enabled,
                    transform: changes.get(&logical_id).map_or(*previous, |transform| {
                        GpuInstance::new(transform.translation, transform.rotation, transform.scale)
                    }),
                })
                .collect();
            Some(InstanceChunk::new(&rows, old.sphere, Some(old)))
        });
        if let Some((chunk, bounds)) = updated {
            draw.chunk = chunk.clone();
            world.entity_mut(entity).insert((draw, *bounds));
        }
    }
    if let Some(picker) = world.get_resource::<GpuPointPicker>() {
        picker.invalidate_representation(false);
    }
}

pub(crate) fn existing_draws(world: &World, instancer: Entity) -> HashMap<DrawKey, Entity> {
    world
        .get::<Children>(instancer)
        .into_iter()
        .flat_map(|children| children.iter())
        .filter_map(|entity| Some((world.get::<UsdInstancedDraw>(entity)?.key, entity)))
        .collect()
}

pub(in crate::route) fn reconcile_draws(
    world: &mut World,
    instancer: Entity,
    instancer_path: Arc<str>,
    prototype: usize,
    meshes: &[PreparedGpuMesh],
    rows: &[PackedRow],
    capacity: usize,
    previous: &mut HashMap<DrawKey, Entity>,
) {
    let sphere = prototype_sphere(world, meshes);
    for (chunk_index, instances) in rows.chunks(capacity).enumerate() {
        let first = DrawKey {
            prototype,
            chunk: chunk_index,
            part: 0,
        };
        let old = previous
            .get(&first)
            .and_then(|entity| world.get::<UsdInstancedDraw>(*entity))
            .map(|draw| &draw.chunk);
        let (chunk, bounds) = InstanceChunk::new(instances, sphere, old);
        for (
            part_index,
            PreparedGpuMesh {
                mesh,
                material,
                part,
                pick_mesh,
            },
        ) in meshes.iter().enumerate()
        {
            let key = DrawKey {
                part: part_index,
                ..first
            };
            let material = world
                .resource::<Assets<StandardMaterial>>()
                .get(material)
                .map(InstancedMaterial::from)
                .unwrap_or(InstancedMaterial::from(&StandardMaterial::default()));
            let draw = UsdInstancedDraw {
                mesh: mesh.clone(),
                part: *part,
                material,
                chunk: chunk.clone(),
                instancer: instancer_path.clone(),
                pick_mesh: pick_mesh.clone(),
                key,
            };
            if let Some(entity) = previous.remove(&key) {
                world.entity_mut(entity).insert((draw, bounds));
            } else {
                world.spawn((draw, bounds, ChildOf(instancer)));
            }
        }
    }
}

/// The bounding sphere of a prototype's meshes, in prototype space.
fn prototype_sphere(world: &World, meshes: &[PreparedGpuMesh]) -> Vec4 {
    let assets = world.resource::<Assets<Mesh>>();
    let (mut min, mut max) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
    for PreparedGpuMesh { mesh, part, .. } in meshes {
        let Some(aabb) = assets.get(mesh).and_then(|mesh| mesh.get_aabb()) else {
            continue;
        };
        let (center, half) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
        for corner in 0..8 {
            let sign = Vec3::new(
                if corner & 1 == 0 { -1.0 } else { 1.0 },
                if corner & 2 == 0 { -1.0 } else { 1.0 },
                if corner & 4 == 0 { -1.0 } else { 1.0 },
            );
            let point = part.transform_point3(center + half * sign);
            min = min.min(point);
            max = max.max(point);
        }
    }
    if !min.is_finite() {
        return Vec4::ZERO;
    }
    let center = (min + max) * 0.5;
    center.extend((max - center).length())
}
