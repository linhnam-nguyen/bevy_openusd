use openusd::sdf::{Path, Value};
use openusd::usd::Stage;

use crate::read::util::{
    connections_at, default_at, read_asset_path, read_token_or_string, read_vec2f,
};

use super::channels::{
    ColourSetter, MATERIALX_STD_SURFACE_CHANNELS, OMNIPBR_CHANNELS, OMNISURFACE_CHANNELS,
    PREVIEW_CHANNELS, ScalarSetter, TextureSetter,
};
use super::types::{ReadPreviewMaterial, UvTransform};

/// Read a `Material` prim and return its decoded surface inputs.
pub fn read_preview_material(
    stage: &Stage,
    material: &Path,
) -> anyhow::Result<Option<ReadPreviewMaterial>> {
    let Some((shader, dialect)) = resolve_surface_shader(stage, material)? else {
        return Ok(None);
    };

    let shader_id = read_token_or_string(stage, &shader, "info:id")?;
    let mdl_subid = read_token_or_string(stage, &shader, "info:mdl:sourceAsset:subIdentifier")?;
    let mdl_source = read_asset_path(stage, &shader, "info:mdl:sourceAsset")?;
    let mdl_basename = mdl_source.as_deref().and_then(|p| {
        std::path::Path::new(p)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    });
    let mdl_id = mdl_subid.as_deref().or(mdl_basename.as_deref());

    let channels: &[(&str, ColourSetter, ScalarSetter, TextureSetter)] = match dialect {
        SurfaceDialect::Mdl => match mdl_id {
            Some("OmniSurface") | Some("OmniSurfaceLite") | Some("OmniSurfaceBase") => {
                OMNISURFACE_CHANNELS
            }
            _ => OMNIPBR_CHANNELS,
        },
        SurfaceDialect::MaterialX => match shader_id.as_deref() {
            Some("ND_standard_surface_surfaceshader") => MATERIALX_STD_SURFACE_CHANNELS,
            _ => PREVIEW_CHANNELS,
        },
        SurfaceDialect::Preview => match shader_id.as_deref() {
            Some("UsdPreviewSurface") | Some("ND_UsdPreviewSurface_surfaceshader") | None => {
                PREVIEW_CHANNELS
            }
            Some("OmniPBR") | Some("OmniPBR_Opacity") | Some("OmniPBR_ClearCoat") => {
                OMNIPBR_CHANNELS
            }
            Some("OmniSurface") | Some("OmniSurfaceLite") | Some("OmniSurfaceBase") => {
                OMNISURFACE_CHANNELS
            }
            Some("ND_standard_surface_surfaceshader") => MATERIALX_STD_SURFACE_CHANNELS,
            _ => return Ok(None),
        },
    };

    let mut out = ReadPreviewMaterial::default();
    for (channel, bind_colour, bind_scalar, bind_texture) in channels {
        let (value, texture) = resolve_channel(stage, material, &shader, channel)?;
        if let Some(tex) = texture {
            bind_texture(&mut out, tex);
        }
        match value {
            Some(ResolvedValue::Color3(c)) => bind_colour(&mut out, c),
            Some(ResolvedValue::Scalar(s)) => bind_scalar(&mut out, s),
            None => {}
        }
    }
    out.uv_transform = read_uv_transform(stage, material)?;
    Ok(Some(out))
}

/// Find a `UsdTransform2d` node in the material's shader network and read its
/// `scale` / `rotation` / `translation` inputs. USD materials generally share a
/// single st transform across textures, so the first one found is applied
/// material-wide. `None` when the network has no transform (identity `st`).
fn read_uv_transform(stage: &Stage, material: &Path) -> anyhow::Result<Option<UvTransform>> {
    for child in stage
        .prim(material.clone())
        .child_names()
        .unwrap_or_default()
    {
        let node = material.append_path(child.as_str())?;
        if read_token_or_string(stage, &node, "info:id")?.as_deref() != Some("UsdTransform2d") {
            continue;
        }
        let mut t = UvTransform::default();
        if let Some(s) = read_vec2f(stage, &node, "inputs:scale")? {
            t.scale = s;
        }
        if let Some(tr) = read_vec2f(stage, &node, "inputs:translation")? {
            t.translation = tr;
        }
        if let Some(Value::Float(r)) = default_at(stage, &node.append_property("inputs:rotation")?)?
        {
            t.rotation_deg = r;
        } else if let Some(Value::Double(r)) =
            default_at(stage, &node.append_property("inputs:rotation")?)?
        {
            t.rotation_deg = r as f32;
        }
        return Ok(Some(t));
    }
    Ok(None)
}

#[derive(Copy, Clone, Debug)]
enum SurfaceDialect {
    Preview,
    MaterialX,
    Mdl,
}

fn resolve_surface_shader(
    stage: &Stage,
    material: &Path,
) -> anyhow::Result<Option<(Path, SurfaceDialect)>> {
    let outputs = [
        ("outputs:surface", SurfaceDialect::Preview),
        ("outputs:mtlx:surface", SurfaceDialect::MaterialX),
        ("outputs:mdl:surface", SurfaceDialect::Mdl),
    ];
    for (attr_name, dialect) in outputs {
        let attr_path = material.append_property(attr_name)?;
        if let Some(t) = connections_at(stage, &attr_path)?.into_iter().next() {
            return Ok(Some((t.prim_path(), dialect)));
        }
    }
    // Fallback: scan child Shader prims and infer the dialect.
    for child in stage
        .prim(material.clone())
        .child_names()
        .unwrap_or_default()
    {
        let shader = material.append_path(child.as_str())?;
        if stage.prim(shader.clone()).type_name()?.as_deref() != Some("Shader") {
            continue;
        }
        let shader_id = read_token_or_string(stage, &shader, "info:id")?;
        let mdl_subid = read_token_or_string(stage, &shader, "info:mdl:sourceAsset:subIdentifier")?;
        let mdl_source = read_asset_path(stage, &shader, "info:mdl:sourceAsset")?;
        if mdl_subid.is_some() || mdl_source.is_some() {
            return Ok(Some((shader, SurfaceDialect::Mdl)));
        }
        if matches!(
            shader_id.as_deref(),
            Some("UsdPreviewSurface")
                | Some("ND_UsdPreviewSurface_surfaceshader")
                | Some("OmniPBR")
                | Some("OmniPBR_Opacity")
                | Some("OmniPBR_ClearCoat")
        ) {
            return Ok(Some((shader, SurfaceDialect::Preview)));
        }
        if matches!(
            shader_id.as_deref(),
            Some("ND_standard_surface_surfaceshader")
        ) {
            return Ok(Some((shader, SurfaceDialect::MaterialX)));
        }
    }
    Ok(None)
}

#[derive(Debug)]
enum ResolvedValue {
    Color3([f32; 3]),
    Scalar(f32),
}

fn resolve_channel(
    stage: &Stage,
    material: &Path,
    shader: &Path,
    channel: &str,
) -> anyhow::Result<(Option<ResolvedValue>, Option<String>)> {
    let mat_attr = format!("inputs:{channel}");
    let mat_path = material.append_property(&mat_attr)?;
    let (v, t) = resolve_attr_chain(stage, &mat_path)?;
    if v.is_some() || t.is_some() {
        return Ok((v, t));
    }
    let sh_path = shader.append_property(&mat_attr)?;
    resolve_attr_chain(stage, &sh_path)
}

fn resolve_attr_chain(
    stage: &Stage,
    attr_path: &Path,
) -> anyhow::Result<(Option<ResolvedValue>, Option<String>)> {
    let mut cur = attr_path.clone();
    for _ in 0..16 {
        if let Some(next) = connections_at(stage, &cur)?.into_iter().next() {
            let prim = next.prim_path();
            match shader_kind(stage, &prim)? {
                ShaderKind::Texture => return Ok((None, read_texture_file(stage, &prim)?)),
                ShaderKind::NormalMap => {
                    cur = prim.append_property("inputs:in")?;
                    continue;
                }
                ShaderKind::Constant => {
                    let v_path = prim.append_property("inputs:value")?;
                    return Ok((default_at(stage, &v_path)?.and_then(value_to_preview), None));
                }
                ShaderKind::Multiply | ShaderKind::AddOrSubtract => {
                    cur = prim.append_property("inputs:in1")?;
                    continue;
                }
                ShaderKind::Mix => {
                    cur = prim.append_property("inputs:fg")?;
                    continue;
                }
                ShaderKind::Unknown => {
                    cur = next;
                    continue;
                }
            }
        }
        let default = default_at(stage, &cur)?;
        match default.clone() {
            Some(Value::AssetPath(s)) => return Ok((None, Some(s.as_str().to_string()))),
            Some(Value::String(s)) => return Ok((None, Some(s))),
            _ => {}
        }
        return Ok((default.and_then(value_to_preview), None));
    }
    Ok((None, None))
}

enum ShaderKind {
    Texture,
    NormalMap,
    Constant,
    Multiply,
    AddOrSubtract,
    Mix,
    Unknown,
}

fn shader_kind(stage: &Stage, prim: &Path) -> anyhow::Result<ShaderKind> {
    let id = read_token_or_string(stage, prim, "info:id")?;
    Ok(match id.as_deref() {
        Some("UsdUVTexture") => ShaderKind::Texture,
        Some(s) if s.starts_with("ND_image_") => ShaderKind::Texture,
        Some("ND_normalmap") => ShaderKind::NormalMap,
        Some(s) if s.starts_with("ND_constant_") => ShaderKind::Constant,
        Some(s) if s.starts_with("ND_multiply_") => ShaderKind::Multiply,
        Some(s) if s.starts_with("ND_add_") || s.starts_with("ND_subtract_") => {
            ShaderKind::AddOrSubtract
        }
        Some(s) if s.starts_with("ND_mix_") => ShaderKind::Mix,
        _ => ShaderKind::Unknown,
    })
}

fn read_texture_file(stage: &Stage, tex_prim: &Path) -> anyhow::Result<Option<String>> {
    read_asset_path(stage, tex_prim, "inputs:file")
}

fn value_to_preview(v: Value) -> Option<ResolvedValue> {
    match v {
        Value::Float(f) => Some(ResolvedValue::Scalar(f)),
        Value::Double(d) => Some(ResolvedValue::Scalar(d as f32)),
        Value::Vec3f(c) => Some(ResolvedValue::Color3([c.x, c.y, c.z])),
        Value::Vec3d(c) => Some(ResolvedValue::Color3([c.x as f32, c.y as f32, c.z as f32])),
        _ => None,
    }
}
