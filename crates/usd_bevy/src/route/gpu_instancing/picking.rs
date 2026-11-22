//! Bounded asynchronous occurrence queries against raster's visible slots.

use std::sync::{Arc, Mutex};

use bevy::mesh::VertexAttributeValues;
use bevy::render::render_resource::MapMode;

use super::*;

#[cfg(test)]
#[path = "../../../tests/unit/route/gpu_point_picking_tests.rs"]
mod tests;

const RESULT_SIZE: u64 = 16;
const MAX_DRAWS: usize = 1024;
const MAX_ROWS_PER_TASK: usize = 65_535 * 64; // WebGPU workgroups-per-dimension bound
pub(in crate::route) const MAX_TRIANGLE_WORK: usize = 16 * 1024 * 1024;

/// One renderer-neutral GPU hit. The viewport owns SceneAnchor translation.
#[derive(Clone, Debug)]
pub struct GpuPointHit {
    pub instancer: Arc<str>,
    pub logical_id: i64,
    pub distance: f32,
}

#[derive(Clone, Debug)]
pub struct GpuPointPickResult {
    pub hit: Option<GpuPointHit>,
    pub visible_rows: u64,
}

/// A failed query is not a geometric miss. The viewport must not treat it as one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuPointPickError {
    SourceUnavailable,
    InvalidWork,
    ReadbackFailed,
    InvalidReadback,
}

#[derive(Clone, Copy)]
struct PickRequest {
    sequence: u64,
    ray: Ray3d,
}

#[derive(Default)]
struct Requests {
    sequence: u64,
    representation_revision: u64,
    view_revision: u64,
    pending: Option<PickRequest>,
    result: Option<(u64, Result<GpuPointPickResult, GpuPointPickError>)>,
    in_flight: bool,
}

/// At most one submitted GPU readback and one latest pending pointer request.
#[derive(Resource, Default, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct GpuPointPicker(Arc<Mutex<Requests>>);

impl GpuPointPicker {
    /// Presentation changes that invalidate the view a user clicked.
    pub fn view_revision(&self) -> u64 {
        self.0.lock().expect("point pick mailbox").view_revision
    }

    pub fn representation_revision(&self) -> u64 {
        self.0
            .lock()
            .expect("point pick mailbox")
            .representation_revision
    }

    pub(crate) fn invalidate_representation(&self, selection_only: bool) {
        let mut requests = self.0.lock().expect("point pick mailbox");
        requests.representation_revision = requests.representation_revision.saturating_add(1);
        if !selection_only {
            requests.view_revision = requests.view_revision.saturating_add(1);
        }
        requests.sequence = requests
            .sequence
            .checked_add(1)
            .expect("point pick sequence exhausted");
        requests.pending = None;
        requests.result = None;
    }

    pub fn request(&self, ray: Ray3d) -> u64 {
        let mut requests = self.0.lock().expect("point pick mailbox");
        requests.sequence = requests
            .sequence
            .checked_add(1)
            .expect("point pick sequence exhausted");
        let sequence = requests.sequence;
        requests.result = None;
        requests.pending = Some(PickRequest { sequence, ray });
        sequence
    }

    pub fn take_result(
        &self,
        sequence: u64,
    ) -> Option<Result<GpuPointPickResult, GpuPointPickError>> {
        let mut requests = self.0.lock().expect("point pick mailbox");
        if requests.result.as_ref()?.0 != sequence {
            return None;
        }
        requests.result.take().map(|(_, result)| result)
    }

    fn start(&self) -> Option<PickRequest> {
        let mut requests = self.0.lock().expect("point pick mailbox");
        if requests.in_flight {
            return None;
        }
        let request = requests.pending.take()?;
        requests.in_flight = true;
        Some(request)
    }

    fn is_current(&self, sequence: u64) -> bool {
        self.0.lock().expect("point pick mailbox").sequence == sequence
    }

    /// Only the render-world traversal may release the in-flight slot. An old
    /// map_async callback writes to its private channel, never to the mailbox.
    fn cancel_in_flight(&self) {
        self.0.lock().expect("point pick mailbox").in_flight = false;
    }

    fn complete(&self, sequence: u64, result: Result<GpuPointPickResult, GpuPointPickError>) {
        let mut requests = self.0.lock().expect("point pick mailbox");
        requests.in_flight = false;
        if sequence == requests.sequence {
            requests.result = Some((sequence, result));
        }
    }
}

pub(super) struct PickMesh {
    vertices: Vec<[f32; 4]>,
    min: Vec3,
    max: Vec3,
}

impl PickMesh {
    pub(super) fn read(mesh: &Mesh) -> Option<Self> {
        let VertexAttributeValues::Float32x3(positions) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)?
        else {
            return None;
        };
        let indices = mesh.indices()?;
        // Bound each shared GPU triangle buffer independently from request work.
        if indices.is_empty() || indices.len() % 3 != 0 || indices.len() > 3 * 262_144 {
            return None;
        }
        let vertices: Option<Vec<_>> = indices
            .iter()
            .map(|i| positions.get(i).map(|p| [p[0], p[1], p[2], 1.0]))
            .collect();
        let aabb = mesh.get_aabb()?;
        Some(Self {
            vertices: vertices?,
            min: aabb.min().into(),
            max: aabb.max().into(),
        })
    }

    pub(super) fn triangles(&self) -> usize {
        self.vertices.len() / 3
    }
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct PickUniform {
    instancer_from_world: [[f32; 4]; 4],
    mesh_from_prototype: [[f32; 4]; 4],
    origin: [f32; 4],
    direction: [f32; 4],
    bounds_min: [f32; 4],
    bounds_max: [f32; 4],
    row_count: u32,
    vertex_count: u32,
    visible_start: u32,
    visible_limit: u32,
}

#[derive(Resource)]
struct PickPipeline {
    layout: BindGroupLayoutDescriptor,
    depth: CachedComputePipelineId,
    id: CachedComputePipelineId,
}

#[derive(Resource, Default)]
struct PickBuffers(HashMap<AssetId<Mesh>, (Arc<PickMesh>, Buffer)>);

struct PickSegment {
    instancer: Arc<str>,
    chunk: Arc<InstanceChunk>,
}

/// Snapshot source identity, transforms and GPU buffers at the beginning of
/// the request. Later render frames must not silently switch to newer chunks.
struct PickDrawSnapshot {
    mesh: AssetId<Mesh>,
    world: Mat4,
    part: Mat4,
    instancer: Arc<str>,
    chunk: Arc<InstanceChunk>,
    pick_mesh: Arc<PickMesh>,
    instances: Buffer,
    visible: Buffer,
    count: Buffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PickTask {
    draw: usize,
    start: u32,
    limit: u32,
}

#[derive(Default)]
struct BatchReport {
    hit: Option<GpuPointHit>,
    visible_chunks: Vec<(u64, u64)>,
}

type BatchOutcome = Result<BatchReport, GpuPointPickError>;
type BatchChannel = Arc<Mutex<Option<BatchOutcome>>>;

struct PickTraversal {
    request: PickRequest,
    draws: Vec<PickDrawSnapshot>,
    draw_cursor: usize,
    visible_cursor: usize,
    nearest: Option<GpuPointHit>,
    visible_chunks: HashMap<u64, u64>,
    waiting: Option<BatchChannel>,
}

impl PickTraversal {
    fn complete(&self) -> bool {
        self.draw_cursor == self.draws.len()
    }

    /// Every submitted task is strictly limited by both draw count and
    /// triangle/visible-row work; huge individual draws are subdivided too.
    fn next_batch(&mut self) -> Result<Vec<PickTask>, GpuPointPickError> {
        let mut tasks = Vec::new();
        let mut remaining_work = MAX_TRIANGLE_WORK;
        while self.draw_cursor < self.draws.len() && tasks.len() < MAX_DRAWS {
            let draw = &self.draws[self.draw_cursor];
            let rows = draw.chunk.instances.len();
            let triangles = draw.pick_mesh.triangles();
            if triangles == 0 || triangles > MAX_TRIANGLE_WORK || rows > u32::MAX as usize {
                return Err(GpuPointPickError::InvalidWork);
            }
            if self.visible_cursor == rows {
                self.draw_cursor += 1;
                self.visible_cursor = 0;
                continue;
            }
            if remaining_work < triangles {
                break;
            }
            let limit = bounded_pick_rows(rows, self.visible_cursor, triangles, remaining_work);
            if limit == 0 {
                return Err(GpuPointPickError::InvalidWork);
            }
            tasks.push(PickTask {
                draw: self.draw_cursor,
                start: self.visible_cursor as u32,
                limit: limit as u32,
            });
            remaining_work -= limit * triangles;
            self.visible_cursor += limit;
        }
        if tasks.is_empty() && !self.complete() {
            return Err(GpuPointPickError::InvalidWork);
        }
        Ok(tasks)
    }

    fn add_batch(&mut self, batch: BatchReport) {
        for (id, rows) in batch.visible_chunks {
            self.visible_chunks.entry(id).or_insert(rows);
        }
        if let Some(hit) = batch.hit {
            let replace = self.nearest.as_ref().is_none_or(|current| {
                hit.distance < current.distance
                    || (hit.distance == current.distance
                        && (hit.instancer.as_ref(), hit.logical_id)
                            < (current.instancer.as_ref(), current.logical_id))
            });
            if replace {
                self.nearest = Some(hit);
            }
        }
    }

    fn result(&self) -> GpuPointPickResult {
        GpuPointPickResult {
            hit: self.nearest.clone(),
            visible_rows: self.visible_chunks.values().copied().sum(),
        }
    }
}

/// A bounded span in the compacted *visible* index buffer, not source rows.
/// A zero result means this batch has exhausted its remaining work budget.
fn bounded_pick_rows(rows: usize, start: usize, triangles: usize, work: usize) -> usize {
    if triangles == 0 || start >= rows || triangles > MAX_TRIANGLE_WORK {
        return 0;
    }
    (rows - start).min(work / triangles).min(MAX_ROWS_PER_TASK)
}

struct SubmittedPick {
    staging: Buffer,
    segments: Vec<PickSegment>,
    channel: BatchChannel,
}

#[derive(Resource, Default)]
struct Readback(Option<SubmittedPick>);

#[derive(Resource, Default)]
struct ActiveTraversal(Option<PickTraversal>);

pub(super) fn setup(app: &mut App) {
    app.init_resource::<GpuPointPicker>()
        .add_plugins(ExtractResourcePlugin::<GpuPointPicker>::default());
    let render = app.sub_app_mut(RenderApp);
    render
        .init_resource::<PickBuffers>()
        .init_resource::<Readback>()
        .init_resource::<ActiveTraversal>()
        .add_systems(RenderStartup, init)
        .add_systems(
            Core3d,
            encode.after(cull_instances).before(Core3dSystems::MainPass),
        )
        .add_systems(Render, map_results.in_set(RenderSystems::Cleanup));
}

fn init(mut commands: Commands, assets: Res<AssetServer>, cache: Res<PipelineCache>) {
    let layout = BindGroupLayoutDescriptor::new(
        "point_instance_pick",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                binding_types::storage_buffer_read_only_sized(false, None),
                binding_types::storage_buffer_read_only_sized(false, None),
                binding_types::storage_buffer_read_only_sized(false, None),
                binding_types::storage_buffer_read_only_sized(false, None),
                binding_types::uniform_buffer_sized(false, None),
                binding_types::storage_buffer_sized(false, None),
            ),
        ),
    );
    let shader = assets.load("embedded://usd_bevy/route/gpu_instancing_pick.wgsl");
    let queue = |entry: &'static str| {
        cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(entry.into()),
            layout: vec![layout.clone()],
            shader: shader.clone(),
            entry_point: Some(entry.into()),
            ..default()
        })
    };
    commands.insert_resource(PickPipeline {
        depth: queue("nearest_distance"),
        id: queue("nearest_id"),
        layout,
    });
}

#[allow(clippy::too_many_arguments)]
fn encode(
    view: ViewQuery<&RenderVisibleEntities>,
    picker: Res<GpuPointPicker>,
    pipeline: Res<PickPipeline>,
    pipelines: Res<PipelineCache>,
    device: Res<RenderDevice>,
    draws: Query<&ExtractedDraw>,
    chunks: Res<GpuChunks>,
    mut geometry: ResMut<PickBuffers>,
    mut readback: ResMut<Readback>,
    mut traversal: ResMut<ActiveTraversal>,
    mut ctx: RenderContext,
) {
    let (Some(depth), Some(id)) = (
        pipelines.get_compute_pipeline(pipeline.depth),
        pipelines.get_compute_pipeline(pipeline.id),
    ) else {
        // No request is claimed until both compute pipelines are ready.
        return;
    };

    if let Some(active) = traversal.0.as_ref()
        && !picker.is_current(active.request.sequence)
    {
        // Old callbacks only write to the abandoned batch channel. They never
        // complete a newer request or reuse its GPU resources.
        traversal.0 = None;
        picker.cancel_in_flight();
    }
    if let Some(active) = traversal.0.as_mut()
        && let Some(channel) = active.waiting.as_ref()
    {
        let ready = channel.lock().expect("point pick batch channel").take();
        match ready {
            None => return,
            Some(Ok(report)) => {
                active.add_batch(report);
                active.waiting = None;
            }
            Some(Err(error)) => {
                let sequence = active.request.sequence;
                traversal.0 = None;
                picker.complete(sequence, Err(error));
                return;
            }
        }
    }

    if traversal.0.is_none() {
        let Some(request) = picker.start() else {
            return;
        };
        let visible = view.into_inner().get::<UsdInstancedDraw>();
        let mut active_draws = Vec::new();
        // Culling rewrites its compacted visible-slot buffers every frame.
        // Freeze source instance values and *ordered* compacted slots once,
        // after cull_instances, for deterministic multi-frame slice coverage.
        // Multipart draws reuse one immutable snapshot per chunk ID.
        let mut frozen_chunks = HashMap::<u64, (Buffer, Buffer, Buffer)>::default();
        for (entity, _) in visible
            .into_iter()
            .flat_map(|visible| visible.iter_visible())
        {
            let Ok(draw) = draws.get(*entity) else {
                picker.complete(request.sequence, Err(GpuPointPickError::SourceUnavailable));
                return;
            };
            let Some(chunk) = chunks.chunks.get(&draw.chunk.id) else {
                picker.complete(request.sequence, Err(GpuPointPickError::SourceUnavailable));
                return;
            };
            let (instance_copy, visible_copy, count_copy) =
                frozen_chunks.entry(draw.chunk.id).or_insert_with(|| {
                    let instance_bytes =
                        (draw.chunk.instances.len() * std::mem::size_of::<GpuInstance>()) as u64;
                    let visible_bytes =
                        (draw.chunk.instances.len() * std::mem::size_of::<u32>()) as u64;
                    let make_snapshot = |label, size: u64| {
                        device.create_buffer(&BufferDescriptor {
                            label: Some(label),
                            size: size.max(4),
                            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        })
                    };
                    let frozen_instances =
                        make_snapshot("point_pick_frozen_instances", instance_bytes);
                    let frozen_visible = make_snapshot("point_pick_frozen_visible", visible_bytes);
                    let frozen_count = make_snapshot("point_pick_frozen_count", 4);
                    let encoder = ctx.command_encoder();
                    if instance_bytes > 0 {
                        encoder.copy_buffer_to_buffer(
                            &chunk.instances,
                            0,
                            &frozen_instances,
                            0,
                            instance_bytes,
                        );
                        encoder.copy_buffer_to_buffer(
                            &chunk.visible,
                            0,
                            &frozen_visible,
                            0,
                            visible_bytes,
                        );
                    }
                    encoder.copy_buffer_to_buffer(&chunk.count, 0, &frozen_count, 0, 4);
                    (frozen_instances, frozen_visible, frozen_count)
                });
            active_draws.push(PickDrawSnapshot {
                mesh: draw.mesh,
                world: draw.world,
                part: draw.part,
                instancer: draw.instancer.clone(),
                chunk: draw.chunk.clone(),
                pick_mesh: draw.pick_mesh.clone(),
                instances: instance_copy.clone(),
                visible: visible_copy.clone(),
                count: count_copy.clone(),
            });
        }
        geometry
            .0
            .retain(|mesh, _| active_draws.iter().any(|draw| draw.mesh == *mesh));
        traversal.0 = Some(PickTraversal {
            request,
            draws: active_draws,
            draw_cursor: 0,
            visible_cursor: 0,
            nearest: None,
            visible_chunks: HashMap::default(),
            waiting: None,
        });
    }

    let active = traversal.0.as_mut().expect("active point pick traversal");
    // next_batch increments the cursor, so check completion only after it has
    // skipped empty draws. Empty visible work is a legitimate geometric miss.
    let tasks = match active.next_batch() {
        Ok(tasks) => tasks,
        Err(error) => {
            let sequence = active.request.sequence;
            traversal.0 = None;
            picker.complete(sequence, Err(error));
            return;
        }
    };
    if tasks.is_empty() {
        let sequence = active.request.sequence;
        let result = active.result();
        bevy::log::debug!(target: "usd_bevy::route::gpu_instancing", sequence,
            visible_rows = result.visible_rows, hit = result.hit.is_some(),
            "point pick readback");
        traversal.0 = None;
        picker.complete(sequence, Ok(result));
        return;
    }

    bevy::log::debug!(target: "usd_bevy::route::gpu_instancing", sequence = active.request.sequence,
        draws = active.draws.len(), tasks = tasks.len(),
        "submitting packed point pick batch");

    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("point_instance_pick_batch_readback"),
        size: tasks.len() as u64 * RESULT_SIZE,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let layout = pipelines.get_bind_group_layout(&pipeline.layout);
    let mut segments = Vec::with_capacity(tasks.len());
    for task in tasks {
        let draw = &active.draws[task.draw];
        let pick = &draw.pick_mesh;
        let triangles = &geometry
            .0
            .entry(draw.mesh)
            .or_insert_with(|| {
                (
                    pick.clone(),
                    device.create_buffer_with_data(&BufferInitDescriptor {
                        label: Some("point_instance_pick_triangles"),
                        contents: bytemuck::cast_slice(pick.vertices.as_slice()),
                        usage: BufferUsages::STORAGE,
                    }),
                )
            })
            .1;
        let uniform = PickUniform {
            instancer_from_world: draw.world.inverse().to_cols_array_2d(),
            mesh_from_prototype: draw.part.inverse().to_cols_array_2d(),
            origin: active.request.ray.origin.extend(1.0).to_array(),
            direction: active
                .request
                .ray
                .direction
                .as_vec3()
                .extend(0.0)
                .to_array(),
            bounds_min: pick.min.extend(0.0).to_array(),
            bounds_max: pick.max.extend(0.0).to_array(),
            row_count: draw.chunk.instances.len() as u32,
            vertex_count: pick.vertices.len() as u32,
            visible_start: task.start,
            visible_limit: task.limit,
        };
        let params = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("point_instance_pick_parameters"),
            contents: bytemuck::bytes_of(&uniform),
            usage: BufferUsages::UNIFORM,
        });
        let result = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("point_instance_pick_result"),
            contents: bytemuck::bytes_of(&[f32::INFINITY.to_bits(), u32::MAX, 0, 0]),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        });
        let bind = device.create_bind_group(
            "point_instance_pick",
            &layout,
            &BindGroupEntries::sequential((
                draw.instances.as_entire_binding(),
                draw.visible.as_entire_binding(),
                draw.count.as_entire_binding(),
                triangles.as_entire_binding(),
                params.as_entire_binding(),
                result.as_entire_binding(),
            )),
        );
        let encoder = ctx.command_encoder();
        for pipeline in [depth, id] {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(task.limit.div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(
            &result,
            0,
            &staging,
            segments.len() as u64 * RESULT_SIZE,
            RESULT_SIZE,
        );
        segments.push(PickSegment {
            instancer: draw.instancer.clone(),
            chunk: draw.chunk.clone(),
        });
    }
    let channel = Arc::new(Mutex::new(None));
    active.waiting = Some(channel.clone());
    readback.0 = Some(SubmittedPick {
        staging,
        segments,
        channel,
    });
}

fn map_results(mut readback: ResMut<Readback>) {
    let Some(SubmittedPick {
        staging,
        segments,
        channel,
    }) = readback.0.take()
    else {
        return;
    };
    let mapped = staging.clone();
    staging.slice(..).map_async(MapMode::Read, move |result| {
        let outcome = if result.is_err() {
            Err(GpuPointPickError::ReadbackFailed)
        } else {
            match mapped.slice(..).get_mapped_range() {
                Err(_) => Err(GpuPointPickError::ReadbackFailed),
                Ok(bytes) => {
                    let data: &[[u32; 4]] = bytemuck::cast_slice(&bytes);
                    let outcome = if data.len() != segments.len() {
                        Err(GpuPointPickError::InvalidReadback)
                    } else {
                        let mut report = BatchReport::default();
                        for (segment, result) in segments.iter().zip(data) {
                            report
                                .visible_chunks
                                .push((segment.chunk.id, u64::from(result[2])));
                            let distance = f32::from_bits(result[0]);
                            if !distance.is_finite() || distance < 0.0 {
                                continue;
                            }
                            let logical_id = result[1].checked_sub(1).and_then(|slot| {
                                segment.chunk.logical_ids.get(slot as usize).copied()
                            });
                            let Some(logical_id) = logical_id else {
                                continue;
                            };
                            let candidate = GpuPointHit {
                                instancer: segment.instancer.clone(),
                                logical_id,
                                distance,
                            };
                            let replace = report.hit.as_ref().is_none_or(|current| {
                                distance < current.distance
                                    || (distance == current.distance
                                        && (candidate.instancer.as_ref(), candidate.logical_id)
                                            < (current.instancer.as_ref(), current.logical_id))
                            });
                            if replace {
                                report.hit = Some(candidate);
                            }
                        }
                        Ok(report)
                    };
                    drop(bytes);
                    mapped.unmap();
                    outcome
                }
            }
        };
        *channel.lock().expect("point pick batch channel") = Some(outcome);
    });
}
