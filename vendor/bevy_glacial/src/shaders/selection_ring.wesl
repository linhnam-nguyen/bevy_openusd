// Animated selection-ring shader — ported from the astrocraft game.
// Renders the annulus as a set of rotating segmented dashes with a
// fine-striped detail pattern inside each dash; pixels outside the
// dashes are `discard`ed so the mesh reads as a spinning necklace
// of light rather than a solid ring.

import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
};

@if(PREPASS_PIPELINE)
import bevy_pbr::{
    prepass::io::{VertexOutput, FragmentOutput},
    deferred::functions::deferred_output,
};
@if(!PREPASS_PIPELINE)
import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};

struct SelectionRingSettings {
    color_r: f32,
    color_g: f32,
    color_b: f32,
    time: f32,
    pulse_speed: f32,
    pulse_count: f32,
    alpha: f32,
    center_x: f32,
    center_z: f32,
    fine_mult: f32,
    _pad2: f32,
    _pad3: f32,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> settings: SelectionRingSettings;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color,
    );

@if(PREPASS_PIPELINE)
    return deferred_output(in, pbr_input);
@else {
    var out: FragmentOutput;

    let norm_angle = in.uv.y;
    let phase = norm_angle * settings.pulse_count * 6.283185 - settings.time * settings.pulse_speed;
    let pulse = step(0.0, sin(phase));
    if pulse < 0.5 {
        discard;
    }

    let fine_phase = phase * settings.fine_mult;
    let fine_pulse = step(0.3, sin(fine_phase));
    if fine_pulse < 0.5 {
        discard;
    }

    let base_color = vec3<f32>(settings.color_r, settings.color_g, settings.color_b);
    out.color = vec4<f32>(base_color, settings.alpha);
    return out;
}
}
