use bevy::prelude::*;

use crate::read::shade::{ReadPreviewMaterial, read_material_binding, read_preview_material};

use super::super::{PrimRoute, RouteCtx};
use super::material_cache::intern_material;
use super::texture_cache::resolve_texture;

/// Maps a bound Material → the entity's [`MeshMaterial3d`].
pub struct MaterialRoute;

/// The prim's decoded preview material, if it has a binding that resolves.
fn material_of(ctx: &RouteCtx) -> Option<(String, ReadPreviewMaterial)> {
    let binding = read_material_binding(ctx.stage, ctx.path).ok().flatten()?;
    let material = read_preview_material(ctx.stage, &binding).ok().flatten()?;
    Some((binding.as_str().to_owned(), material))
}

pub(super) fn build_standard_material(
    read: &ReadPreviewMaterial,
    world: &mut World,
) -> StandardMaterial {
    let mut m = StandardMaterial::default();
    if let Some(c) = read.diffuse_color {
        let a = read.opacity.unwrap_or(1.0);
        m.base_color = Color::srgba(c[0], c[1], c[2], a);
    } else if let Some(a) = read.opacity {
        m.base_color.set_alpha(a);
    }
    if read.opacity.is_some_and(|a| a < 1.0) {
        m.alpha_mode = AlphaMode::Blend;
    }
    if let Some(r) = read.roughness {
        m.perceptual_roughness = r;
    }
    if let Some(mtl) = read.metallic {
        m.metallic = mtl;
    }
    if let Some(e) = read.emissive_color {
        m.emissive = LinearRgba::rgb(e[0], e[1], e[2]);
    }
    if let Some(ior) = read.ior {
        m.ior = ior;
    }
    if let Some(uv) = &read.uv_transform {
        m.uv_transform = bevy::math::Affine2::from_scale_angle_translation(
            Vec2::from(uv.scale),
            uv.rotation_deg.to_radians(),
            Vec2::from(uv.translation),
        );
    }

    // Resolve textures from cache/disk/usdz
    if let Some(p) = &read.diffuse_texture {
        m.base_color_texture = resolve_texture(world, p, true);
    }
    if let Some(p) = &read.normal_texture {
        m.normal_map_texture = resolve_texture(world, p, false);
    }
    if let Some(p) = &read.metallic_texture {
        m.metallic_roughness_texture = resolve_texture(world, p, false);
    }
    if let Some(p) = &read.emissive_texture {
        m.emissive_texture = resolve_texture(world, p, true);
    }
    if let Some(p) = &read.occlusion_texture {
        m.occlusion_texture = resolve_texture(world, p, false);
    }

    m
}

impl PrimRoute for MaterialRoute {
    fn matches(&self, ctx: &RouteCtx) -> bool {
        read_material_binding(ctx.stage, ctx.path)
            .ok()
            .flatten()
            .is_some()
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        if world.get_resource::<Assets<StandardMaterial>>().is_none() {
            return;
        }
        let Some((binding, descriptor)) = material_of(ctx) else {
            return;
        };
        let Some(handle) = intern_material(world, &binding, &descriptor) else {
            return;
        };
        if let Some(mut mat) = world.get_mut::<MeshMaterial3d<StandardMaterial>>(entity) {
            mat.0 = handle;
        } else if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(MeshMaterial3d(handle));
        }
    }
}
