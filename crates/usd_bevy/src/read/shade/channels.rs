use super::types::ReadPreviewMaterial;

pub(super) type ColourSetter = fn(&mut ReadPreviewMaterial, [f32; 3]);
pub(super) type ScalarSetter = fn(&mut ReadPreviewMaterial, f32);
pub(super) type TextureSetter = fn(&mut ReadPreviewMaterial, String);

fn set_diffuse_c(o: &mut ReadPreviewMaterial, c: [f32; 3]) {
    o.diffuse_color = Some(c);
}
fn set_diffuse_s(_: &mut ReadPreviewMaterial, _: f32) {}
fn set_diffuse_tex(o: &mut ReadPreviewMaterial, s: String) {
    o.diffuse_texture = Some(s);
}
fn set_opacity_c(_: &mut ReadPreviewMaterial, _: [f32; 3]) {}
fn set_opacity_s(o: &mut ReadPreviewMaterial, s: f32) {
    o.opacity = Some(s);
}
fn set_opacity_tex(o: &mut ReadPreviewMaterial, s: String) {
    o.opacity_texture = Some(s);
}
fn set_opacity_threshold_c(_: &mut ReadPreviewMaterial, _: [f32; 3]) {}
fn set_opacity_threshold_s(o: &mut ReadPreviewMaterial, s: f32) {
    o.opacity_threshold = Some(s);
}
fn set_opacity_threshold_tex(_: &mut ReadPreviewMaterial, _: String) {}
fn set_rough_c(_: &mut ReadPreviewMaterial, _: [f32; 3]) {}
fn set_rough_s(o: &mut ReadPreviewMaterial, s: f32) {
    o.roughness = Some(s);
}
fn set_rough_tex(o: &mut ReadPreviewMaterial, s: String) {
    o.roughness_texture = Some(s);
}
fn set_metal_c(_: &mut ReadPreviewMaterial, _: [f32; 3]) {}
fn set_metal_s(o: &mut ReadPreviewMaterial, s: f32) {
    o.metallic = Some(s);
}
fn set_metal_tex(o: &mut ReadPreviewMaterial, s: String) {
    o.metallic_texture = Some(s);
}
fn set_emissive_c(o: &mut ReadPreviewMaterial, c: [f32; 3]) {
    o.emissive_color = Some(c);
}
fn set_emissive_s(_: &mut ReadPreviewMaterial, _: f32) {}
fn set_emissive_tex(o: &mut ReadPreviewMaterial, s: String) {
    o.emissive_texture = Some(s);
}
fn set_ior_c(_: &mut ReadPreviewMaterial, _: [f32; 3]) {}
fn set_ior_s(o: &mut ReadPreviewMaterial, s: f32) {
    o.ior = Some(s);
}
fn set_ior_tex(_: &mut ReadPreviewMaterial, _: String) {}
fn set_normal_c(_: &mut ReadPreviewMaterial, _: [f32; 3]) {}
fn set_normal_s(_: &mut ReadPreviewMaterial, _: f32) {}
fn set_normal_tex(o: &mut ReadPreviewMaterial, s: String) {
    o.normal_texture = Some(s);
}
fn set_occlusion_c(_: &mut ReadPreviewMaterial, _: [f32; 3]) {}
fn set_occlusion_s(_: &mut ReadPreviewMaterial, _: f32) {}
fn set_occlusion_tex(o: &mut ReadPreviewMaterial, s: String) {
    o.occlusion_texture = Some(s);
}

/// MaterialX `opacity` is a `color3`; fold to a luminance scalar.
fn set_opacity_mtlx_c(o: &mut ReadPreviewMaterial, c: [f32; 3]) {
    o.opacity = Some(0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]);
}

pub(super) const PREVIEW_CHANNELS: &[(&str, ColourSetter, ScalarSetter, TextureSetter)] = &[
    (
        "diffuseColor",
        set_diffuse_c,
        set_diffuse_s,
        set_diffuse_tex,
    ),
    ("opacity", set_opacity_c, set_opacity_s, set_opacity_tex),
    (
        "opacityThreshold",
        set_opacity_threshold_c,
        set_opacity_threshold_s,
        set_opacity_threshold_tex,
    ),
    ("roughness", set_rough_c, set_rough_s, set_rough_tex),
    ("metallic", set_metal_c, set_metal_s, set_metal_tex),
    (
        "emissiveColor",
        set_emissive_c,
        set_emissive_s,
        set_emissive_tex,
    ),
    ("ior", set_ior_c, set_ior_s, set_ior_tex),
    ("normal", set_normal_c, set_normal_s, set_normal_tex),
    (
        "occlusion",
        set_occlusion_c,
        set_occlusion_s,
        set_occlusion_tex,
    ),
];

pub(super) const MATERIALX_STD_SURFACE_CHANNELS: &[(
    &str,
    ColourSetter,
    ScalarSetter,
    TextureSetter,
)] = &[
    ("base_color", set_diffuse_c, set_diffuse_s, set_diffuse_tex),
    ("metalness", set_metal_c, set_metal_s, set_metal_tex),
    (
        "specular_roughness",
        set_rough_c,
        set_rough_s,
        set_rough_tex,
    ),
    (
        "emission_color",
        set_emissive_c,
        set_emissive_s,
        set_emissive_tex,
    ),
    (
        "opacity",
        set_opacity_mtlx_c,
        set_opacity_s,
        set_opacity_tex,
    ),
    ("normal", set_normal_c, set_normal_s, set_normal_tex),
];

pub(super) const OMNIPBR_CHANNELS: &[(&str, ColourSetter, ScalarSetter, TextureSetter)] = &[
    (
        "diffuse_color_constant",
        set_diffuse_c,
        set_diffuse_s,
        set_diffuse_tex,
    ),
    (
        "diffuse_texture",
        set_diffuse_c,
        set_diffuse_s,
        set_diffuse_tex,
    ),
    (
        "reflection_roughness_constant",
        set_rough_c,
        set_rough_s,
        set_rough_tex,
    ),
    (
        "reflectionroughness_texture",
        set_rough_c,
        set_rough_s,
        set_rough_tex,
    ),
    ("metallic_constant", set_metal_c, set_metal_s, set_metal_tex),
    ("metallic_texture", set_metal_c, set_metal_s, set_metal_tex),
    (
        "emissive_color",
        set_emissive_c,
        set_emissive_s,
        set_emissive_tex,
    ),
    (
        "emissive_color_texture",
        set_emissive_c,
        set_emissive_s,
        set_emissive_tex,
    ),
    (
        "opacity_constant",
        set_opacity_c,
        set_opacity_s,
        set_opacity_tex,
    ),
    (
        "opacity_texture",
        set_opacity_c,
        set_opacity_s,
        set_opacity_tex,
    ),
    (
        "normalmap_texture",
        set_normal_c,
        set_normal_s,
        set_normal_tex,
    ),
];

pub(super) const OMNISURFACE_CHANNELS: &[(&str, ColourSetter, ScalarSetter, TextureSetter)] = &[
    (
        "diffuse_reflection_color",
        set_diffuse_c,
        set_diffuse_s,
        set_diffuse_tex,
    ),
    (
        "diffuse_reflection_color_image",
        set_diffuse_c,
        set_diffuse_s,
        set_diffuse_tex,
    ),
    (
        "geometry_normal_image",
        set_normal_c,
        set_normal_s,
        set_normal_tex,
    ),
    (
        "geometry_opacity_image",
        set_opacity_c,
        set_opacity_s,
        set_opacity_tex,
    ),
    (
        "geometry_opacity",
        set_opacity_c,
        set_opacity_s,
        set_opacity_tex,
    ),
    ("roughness", set_rough_c, set_rough_s, set_rough_tex),
    ("metalness", set_metal_c, set_metal_s, set_metal_tex),
    (
        "emission_color",
        set_emissive_c,
        set_emissive_s,
        set_emissive_tex,
    ),
    (
        "emission_color_image",
        set_emissive_c,
        set_emissive_s,
        set_emissive_tex,
    ),
];
