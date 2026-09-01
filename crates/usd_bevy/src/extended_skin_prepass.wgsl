// Pass-specific prepass/shadow vertex path for the fixed-16 skinning material.
// Its input locations mirror Bevy's prepass layout, while the four extended
// influence groups remain available at locations 8 through 13.
#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    morph::{morph_normal, morph_position, morph_tangent},
    prepass_io::VertexOutput,
    skinning,
    view_transformations::position_world_to_clip,
}

struct ExtendedPrepassVertex {
    @builtin(instance_index) instance_index: u32,
#ifdef VERTEX_POSITIONS
    @location(0) position: vec3<f32>,
#endif
#ifdef VERTEX_UVS_A
    @location(1) uv: vec2<f32>,
#endif
#ifdef VERTEX_UVS_B
    @location(2) uv_b: vec2<f32>,
#endif
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
    @location(3) normal: vec3<f32>,
#endif
#ifdef VERTEX_TANGENTS
    @location(4) tangent: vec4<f32>,
#endif
#endif
    @location(5) joint_indices: vec4<u32>,
    @location(6) joint_weights: vec4<f32>,
#ifdef VERTEX_COLORS
    @location(7) color: vec4<f32>,
#endif
    @location(8) joint_indices_1: vec4<u32>,
    @location(9) joint_weights_1: vec4<f32>,
    @location(10) joint_indices_2: vec4<u32>,
    @location(11) joint_weights_2: vec4<f32>,
    @location(12) joint_indices_3: vec4<u32>,
    @location(13) joint_weights_3: vec4<f32>,
#ifdef MORPH_TARGETS
    @builtin(vertex_index) index: u32,
#endif
};

#ifdef MORPH_TARGETS
fn morph_vertex(vertex_in: ExtendedPrepassVertex, instance_index: u32) -> ExtendedPrepassVertex {
    var vertex = vertex_in;
    let first_vertex = mesh[instance_index].first_vertex_index;
    let vertex_index = vertex.index - first_vertex;
    let weight_count = bevy_pbr::morph::layer_count(instance_index);
    for (var i: u32 = 0u; i < weight_count; i ++) {
        let weight = bevy_pbr::morph::weight_at(i, instance_index);
        if weight == 0.0 {
            continue;
        }
        vertex.position += weight * morph_position(vertex_index, i, instance_index);
#ifdef VERTEX_NORMALS
        vertex.normal += weight * morph_normal(vertex_index, i, instance_index);
#endif
#ifdef VERTEX_TANGENTS
        vertex.tangent += vec4(weight * morph_tangent(vertex_index, i, instance_index), 0.0);
#endif
    }
    return vertex;
}

fn morph_prev_vertex(
    vertex_in: ExtendedPrepassVertex,
    instance_index: u32,
) -> ExtendedPrepassVertex {
    var vertex = vertex_in;
    let first_vertex = mesh[instance_index].first_vertex_index;
    let vertex_index = vertex.index - first_vertex;
    let weight_count = bevy_pbr::morph::layer_count(instance_index);
    for (var i: u32 = 0u; i < weight_count; i ++) {
        let weight = bevy_pbr::morph::prev_weight_at(i, instance_index);
        if weight == 0.0 {
            continue;
        }
        vertex.position += weight * morph_position(vertex_index, i, instance_index);
    }
    return vertex;
}
#endif

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
#ifdef MORPH_TARGETS
    var vertex = morph_vertex(vertex_no_morph, vertex_no_morph.instance_index);
#else
    var vertex = vertex_no_morph;
#endif

    let mesh_world_from_local = mesh_functions::get_world_from_local(vertex_no_morph.instance_index);
    let world_from_local = extended_skin_model(vertex, vertex_no_morph.instance_index);

    out.world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );
    out.position = position_world_to_clip(out.world_position.xyz);
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
#endif

#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif

#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
    out.world_normal = skinning::skin_normals(world_from_local, vertex.normal);
#endif
#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local,
        vertex.tangent,
        vertex_no_morph.instance_index,
    );
#endif
#endif

#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif

#ifdef MOTION_VECTOR_PREPASS
#ifdef MORPH_TARGETS
#ifdef HAS_PREVIOUS_MORPH
    let prev_vertex = morph_prev_vertex(vertex_no_morph, vertex_no_morph.instance_index);
#else
    let prev_vertex = vertex_no_morph;
#endif
#else
    let prev_vertex = vertex_no_morph;
#endif

#ifdef HAS_PREVIOUS_SKIN
    let prev_model = extended_skin_prev_model(prev_vertex, vertex_no_morph.instance_index);
#else
    let prev_model = mesh_functions::get_previous_world_from_local(vertex_no_morph.instance_index);
#endif
    out.previous_world_position = mesh_functions::mesh_position_local_to_world(
        prev_model,
        vec4<f32>(prev_vertex.position, 1.0),
    );
#endif

#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex_no_morph.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex_no_morph.instance_index,
        mesh_world_from_local[3],
    );
#endif

    return out;
}
