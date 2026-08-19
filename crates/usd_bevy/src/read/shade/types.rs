use openusd::sdf::Path;
use openusd::usd::Stage;

use crate::read::util::targets_at;

/// Decoded UsdPreviewSurface material. Each channel is `None` (unauthored),
/// a scalar, or a texture asset path (caller resolves via the AssetServer).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadPreviewMaterial {
    pub diffuse_color: Option<[f32; 3]>,
    pub opacity: Option<f32>,
    pub opacity_threshold: Option<f32>,
    pub roughness: Option<f32>,
    pub metallic: Option<f32>,
    pub emissive_color: Option<[f32; 3]>,
    pub ior: Option<f32>,

    pub diffuse_texture: Option<String>,
    pub normal_texture: Option<String>,
    pub roughness_texture: Option<String>,
    pub metallic_texture: Option<String>,
    pub opacity_texture: Option<String>,
    pub emissive_texture: Option<String>,
    pub occlusion_texture: Option<String>,

    /// `UsdTransform2d` on the texture-coordinate chain (scale/rotate/translate
    /// of `st`), if the network has one. Applied to `StandardMaterial::uv_transform`.
    pub uv_transform: Option<UvTransform>,
}

/// A 2D texture-coordinate transform read from a `UsdTransform2d` node:
/// USD applies it as `st' = rotate(scale * st) + translation`, with rotation in
/// degrees, counter-clockwise about the origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvTransform {
    pub translation: [f32; 2],
    pub rotation_deg: f32,
    pub scale: [f32; 2],
}

impl Default for UvTransform {
    fn default() -> Self {
        Self {
            translation: [0.0, 0.0],
            rotation_deg: 0.0,
            scale: [1.0, 1.0],
        }
    }
}

/// Read `material:binding` on a geom prim and return the bound Material prim
/// path. `None` if no binding is authored.
pub fn read_material_binding(stage: &Stage, prim: &Path) -> anyhow::Result<Option<Path>> {
    let rel_path = prim.append_property("material:binding")?;
    Ok(targets_at(stage, &rel_path)?.into_iter().next())
}
