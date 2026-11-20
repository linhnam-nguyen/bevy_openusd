// Pass-specific prepass/shadow vertex path for the fixed-16 skinning material.
// Its input locations mirror Bevy's prepass layout, while the four extended
// influence groups remain available at locations 8 through 13.
import bevy_pbr::render::{
    mesh_bindings::mesh,
    mesh_functions,
    morph,
    morph::{morph_normal, morph_position, morph_tangent},
    skinning,
    view_transformations::position_world_to_clip,
};
import bevy_pbr::prepass::io::VertexOutput;

struct ExtendedPrepassVertex {
    @builtin(instance_index) instance_index: u32,
@if(VERTEX_POSITIONS)
    @location(0) position: vec3<f32>,
@if(VERTEX_UVS_A)
    @location(1) uv: vec2<f32>,
@if(VERTEX_UVS_B)
    @location(2) uv_b: vec2<f32>,
@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_NORMALS)
    @location(3) normal: vec3<f32>,
@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_TANGENTS)
    @location(4) tangent: vec4<f32>,
    @location(5) joint_indices: vec4<u32>,
    @location(6) joint_weights: vec4<f32>,
@if(VERTEX_COLORS)
    @location(7) color: vec4<f32>,
    @location(8) joint_indices_1: vec4<u32>,
    @location(9) joint_weights_1: vec4<f32>,
    @location(10) joint_indices_2: vec4<u32>,
    @location(11) joint_weights_2: vec4<f32>,
    @location(12) joint_indices_3: vec4<u32>,
    @location(13) joint_weights_3: vec4<f32>,
@if(MORPH_TARGETS)
    @builtin(vertex_index) index: u32,
};

@if(MORPH_TARGETS)
fn morph_vertex(vertex_in: ExtendedPrepassVertex, instance_index: u32) -> ExtendedPrepassVertex {
    var vertex = vertex_in;
    let first_vertex = mesh[instance_index].first_vertex_index;
    let vertex_index = vertex.index - first_vertex;
    let morph_descriptor_index = mesh[instance_index].morph_descriptor_index;
    let weight_count = morph::layer_count(morph_descriptor_index);
    for (var i: u32 = 0u; i < weight_count; i ++) {
        let weight = morph::weight_at(i, morph_descriptor_index);
        if weight == 0.0 {
            continue;
        }
        vertex.position += weight * morph_position(vertex_index, i, morph_descriptor_index);
@if(VERTEX_NORMALS)
        vertex.normal += weight * morph_normal(vertex_index, i, morph_descriptor_index);
@if(VERTEX_TANGENTS)
        vertex.tangent += vec4(weight * morph_tangent(vertex_index, i, morph_descriptor_index), 0.0);
    }
    return vertex;
}

@if(MORPH_TARGETS)
fn morph_prev_vertex(
    vertex_in: ExtendedPrepassVertex,
    instance_index: u32,
) -> ExtendedPrepassVertex {
    var vertex = vertex_in;
    let first_vertex = mesh[instance_index].first_vertex_index;
    let vertex_index = vertex.index - first_vertex;
    let morph_descriptor_index = mesh[instance_index].morph_descriptor_index;
    let weight_count = morph::layer_count(morph_descriptor_index);
    for (var i: u32 = 0u; i < weight_count; i ++) {
        let weight = morph::prev_weight_at(i, morph_descriptor_index);
        if weight == 0.0 {
            continue;
        }
        vertex.position += weight * morph_position(vertex_index, i, morph_descriptor_index);
    }
    return vertex;
}

fn extended_skin_model(vertex: ExtendedPrepassVertex, instance_index: u32) -> mat4x4<f32> {
    return skinning::skin_model(vertex.joint_indices, vertex.joint_weights, instance_index)
        + skinning::skin_model(vertex.joint_indices_1, vertex.joint_weights_1, instance_index)
        + skinning::skin_model(vertex.joint_indices_2, vertex.joint_weights_2, instance_index)
        + skinning::skin_model(vertex.joint_indices_3, vertex.joint_weights_3, instance_index);
}

fn extended_skin_prev_model(vertex: ExtendedPrepassVertex, instance_index: u32) -> mat4x4<f32> {
    return skinning::skin_prev_model(vertex.joint_indices, vertex.joint_weights, instance_index)
        + skinning::skin_prev_model(vertex.joint_indices_1, vertex.joint_weights_1, instance_index)
        + skinning::skin_prev_model(vertex.joint_indices_2, vertex.joint_weights_2, instance_index)
        + skinning::skin_prev_model(vertex.joint_indices_3, vertex.joint_weights_3, instance_index);
}

@vertex
fn vertex(vertex_no_morph: ExtendedPrepassVertex) -> VertexOutput {
    var out: VertexOutput;
@if(MORPH_TARGETS)
    var vertex = morph_vertex(vertex_no_morph, vertex_no_morph.instance_index);
@else
    var vertex = vertex_no_morph;

    let mesh_world_from_local = mesh_functions::get_world_from_local(vertex_no_morph.instance_index);
    let world_from_local = extended_skin_model(vertex, vertex_no_morph.instance_index);

    out.world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );
    out.position = position_world_to_clip(out.world_position.xyz);
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
}

@if(VERTEX_UVS_A)
    out.uv = vertex.uv;
@if(VERTEX_UVS_B)
    out.uv_b = vertex.uv_b;

@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS) {
@if(VERTEX_NORMALS)
    out.world_normal = skinning::skin_normals(world_from_local, vertex.normal);
@if(VERTEX_TANGENTS)
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local,
        vertex.tangent,
        vertex_no_morph.instance_index,
    );
}

@if(VERTEX_COLORS)
    out.color = vertex.color;

@if(MOTION_VECTOR_PREPASS) {
@if(MORPH_TARGETS && HAS_PREVIOUS_MORPH)
    let prev_vertex = morph_prev_vertex(vertex_no_morph, vertex_no_morph.instance_index);
@else
    let prev_vertex = vertex_no_morph;

@if(HAS_PREVIOUS_SKIN)
    let prev_model = extended_skin_prev_model(prev_vertex, vertex_no_morph.instance_index);
@else
    let prev_model = mesh_functions::get_previous_world_from_local(vertex_no_morph.instance_index);

    out.previous_world_position = mesh_functions::mesh_position_local_to_world(
        prev_model,
        vec4<f32>(prev_vertex.position, 1.0),
    );
}

@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_no_morph.instance_index;
@if(VISIBILITY_RANGE_DITHER)
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex_no_morph.instance_index,
        mesh_world_from_local[3],
    );

    return out;
}
