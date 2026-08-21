use bevy::prelude::*;

use crate::read::shade::{
    ReadPreviewMaterial, material_network_dependencies, read_material_binding,
    read_preview_material,
};

use super::super::{PrimRoute, RouteCtx};
use super::consumers::MaterialConsumerIndex;
use super::material_cache::intern_material;
use super::texture_cache::resolve_texture;

/// Maps a bound Material → the entity's [`MeshMaterial3d`].
pub struct MaterialRoute;

/// The prim's decoded preview material, if it has a binding that resolves.
fn material_of(ctx: &RouteCtx, world: &mut World) -> Option<(String, ReadPreviewMaterial)> {
    let binding = read_material_binding(ctx.stage, ctx.path).ok().flatten()?;
    super::record_descriptor_read(world);
    let material = read_preview_material(ctx.stage, &binding).ok().flatten()?;
    Some((binding.as_str().to_owned(), material))
}

fn is_material_network_prim(ctx: &RouteCtx) -> bool {
    matches!(ctx.type_name.as_deref(), Some("Material" | "Shader"))
}

fn material_patch_relevant(ctx: &RouteCtx, changed: &[&str]) -> bool {
    if changed.is_empty() || changed.iter().any(|p| p.starts_with("material:binding")) {
        return true;
    }
    if !is_material_network_prim(ctx) {
        return false;
    }
    changed.iter().any(|property| {
        property.starts_with("inputs:")
            || property.starts_with("outputs:")
            || property.starts_with("info:")
    })
}

fn apply_bound_material(ctx: &RouteCtx, world: &mut World, entity: Entity) {
    let binding = read_material_binding(ctx.stage, ctx.path)
        .ok()
        .flatten()
        .map(|path| path.as_str().to_owned());
    if world.get_resource::<MaterialConsumerIndex>().is_none() {
        world.init_resource::<MaterialConsumerIndex>();
    }
    let dependencies = binding.as_deref().map(|material| {
        let Ok(material_path) = openusd::sdf::path(material) else {
            return vec![material.to_owned()];
        };
        material_network_dependencies(ctx.stage, &material_path)
            .unwrap_or_else(|_| vec![material.to_owned()])
    });
    world.resource_mut::<MaterialConsumerIndex>().update(
        ctx.prim_str(),
        binding.as_deref(),
        dependencies.as_deref().unwrap_or(&[]),
        entity,
    );

    let Some((binding, descriptor)) = material_of(ctx, world) else {
        return;
    };
    let Some(handle) = intern_material(world, &binding, &descriptor) else {
        return;
    };
    {
        if let Some(mut mat) = world.get_mut::<MeshMaterial3d<StandardMaterial>>(entity) {
            mat.0 = handle;
        } else if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(MeshMaterial3d(handle));
        }
    }
}

fn reproject_consumers(ctx: &RouteCtx, world: &mut World) {
    let entries = world
        .get_resource::<MaterialConsumerIndex>()
        .map(|index| index.consumer_entities_for(ctx.prim_str()))
        .unwrap_or_default();
    for (path, entity) in entries {
        if world.get_entity(entity).is_err() {
            if let Some(mut index) = world.get_resource_mut::<MaterialConsumerIndex>() {
                index.remove_consumer(&path);
            }
            continue;
        }
        let Ok(path) = openusd::sdf::path(&path) else {
            continue;
        };
        let consumer_ctx = RouteCtx::at(ctx.stage, &path, ctx.time);
        apply_bound_material(&consumer_ctx, world, entity);
    }
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
    fn telemetry_key(&self) -> Option<&'static str> {
        Some("material")
    }

    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.is_some()
            || read_material_binding(ctx.stage, ctx.path)
                .ok()
                .flatten()
                .is_some()
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        if world.get_resource::<Assets<StandardMaterial>>().is_none() {
            return;
        }
        apply_bound_material(ctx, world, entity);
    }

    fn patch(&self, ctx: &RouteCtx, world: &mut World, entity: Entity, changed: &[&str]) {
        if !material_patch_relevant(ctx, changed) {
            return;
        }
        if read_material_binding(ctx.stage, ctx.path)
            .ok()
            .flatten()
            .is_some()
        {
            apply_bound_material(ctx, world, entity);
        }
        if is_material_network_prim(ctx) {
            reproject_consumers(ctx, world);
        }
    }
}
