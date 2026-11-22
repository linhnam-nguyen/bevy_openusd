// Integer occurrence picking over the same compacted rows that feed raster.
struct Instance {
    tx: f32,
    ty: f32,
    tz: f32,
    rotation_xy: u32,
    rotation_zw: u32,
    sx: f32,
    sy: f32,
    sz: f32,
}

struct Request {
    instancer_from_world: mat4x4<f32>,
    mesh_from_prototype: mat4x4<f32>,
    origin: vec4<f32>,
    direction: vec4<f32>,
    bounds_min: vec4<f32>,
    bounds_max: vec4<f32>,
    row_count: u32,
    vertex_count: u32,
    visible_start: u32,
    visible_limit: u32,
}

struct Result {
    distance_bits: atomic<u32>,
    source_slot: atomic<u32>,
    visible_rows: atomic<u32>,
    pad: u32,
}

@group(0) @binding(0) var<storage, read> instances: array<Instance>;
@group(0) @binding(1) var<storage, read> visible: array<u32>;
@group(0) @binding(2) var<storage, read> visible_count: u32;
@group(0) @binding(3) var<storage, read> triangles: array<vec4<f32>>;
@group(0) @binding(4) var<uniform> params: Request;
@group(0) @binding(5) var<storage, read_write> result: Result;

const MISS: u32 = 0x7f800000u;

fn rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
}

fn intersects_bounds(origin: vec3<f32>, direction: vec3<f32>) -> bool {
    var lower = 0.0;
    var upper = bitcast<f32>(MISS);
    for (var axis = 0u; axis < 3u; axis = axis + 1u) {
        let d = direction[axis];
        let o = origin[axis];
        if abs(d) < 1e-20 {
            if o < params.bounds_min[axis] || o > params.bounds_max[axis] {
                return false;
            }
            continue;
        }
        let a = (params.bounds_min[axis] - o) / d;
        let b = (params.bounds_max[axis] - o) / d;
        lower = max(lower, min(a, b));
        upper = min(upper, max(a, b));
        if upper < lower {
            return false;
        }
    }
    return true;
}

fn mesh_distance(slot: u32) -> u32 {
    let row = instances[slot];
    let quaternion = normalize(vec4<f32>(unpack2x16snorm(row.rotation_xy), unpack2x16snorm(row.rotation_zw)));
    let inverse_rotation = vec4<f32>(-quaternion.xyz, quaternion.w);
    let offset = vec3<f32>(row.tx, row.ty, row.tz);
    let scale = vec3<f32>(row.sx, row.sy, row.sz);
    let origin = (params.mesh_from_prototype * vec4<f32>(
        rotate(inverse_rotation, (params.instancer_from_world * params.origin).xyz - offset) / scale, 1.0)).xyz;
    let direction = (params.mesh_from_prototype * vec4<f32>(
        rotate(inverse_rotation, (params.instancer_from_world * params.direction).xyz) / scale, 0.0)).xyz;
    if !intersects_bounds(origin, direction) {
        return MISS;
    }
    var nearest = bitcast<f32>(MISS);
    for (var i = 0u; i < params.vertex_count; i = i + 3u) {
        let a = triangles[i].xyz;
        let ab = triangles[i + 1u].xyz - a;
        let ac = triangles[i + 2u].xyz - a;
        let p = cross(direction, ac);
        let det = dot(ab, p);
        if det <= 1e-8 {
            continue;
        }
        let translated = origin - a;
        let u = dot(translated, p) / det;
        let q = cross(translated, ab);
        let v = dot(direction, q) / det;
        if u < 0.0 || v < 0.0 || u + v > 1.0 {
            continue;
        }
        let d = dot(ac, q) / det;
        if d >= 0.0 {
            nearest = min(nearest, d);
        }
    }
    return bitcast<u32>(nearest);
}

@compute @workgroup_size(64)
fn nearest_distance(@builtin(global_invocation_id) global: vec3<u32>) {
    let count = min(visible_count, params.row_count);
    if global.x == 0u {
        atomicStore(&result.visible_rows, count);
    }
    let start = min(params.visible_start, count);
    let size = min(params.visible_limit, count - start);
    if global.x < size {
        atomicMin(&result.distance_bits, mesh_distance(visible[start + global.x]));
    }
}

@compute @workgroup_size(64)
fn nearest_id(@builtin(global_invocation_id) global: vec3<u32>) {
    let distance = atomicLoad(&result.distance_bits);
    let count = min(visible_count, params.row_count);
    let start = min(params.visible_start, count);
    let size = min(params.visible_limit, count - start);
    if distance == MISS || global.x >= size {
        return;
    }
    let slot = visible[start + global.x];
    if mesh_distance(slot) == distance {
        atomicMin(&result.source_slot, slot + 1u);
    }
}
