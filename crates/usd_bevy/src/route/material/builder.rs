use bevy::prelude::*;

use crate::read::shade::{
    ReadPreviewMaterial, material_network_dependencies, read_material_binding,
    read_preview_material,
};

use super::super::{PrimRoute, ProjectionSeed, RouteCtx, mark_render_projection_dirty};
use super::consumers::MaterialConsumerIndex;
use super::material_cache::{InitialMaterialMemo, InitialMaterialRead, intern_material};
use super::provenance::MaterialProjectionProvenance;
use super::texture_cache::resolve_texture;

/// Maps a bound Material → the entity's [`MeshMaterial3d`].
pub struct MaterialRoute;

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
    let binding_path = read_material_binding(ctx.stage, ctx.path).ok().flatten();
    let binding = binding_path.as_ref().map(|path| path.as_str().to_owned());
    if world
        .get_resource::<MaterialProjectionProvenance>()
        .is_none()
    {
        world.init_resource::<MaterialProjectionProvenance>();
    }
    if world.get_resource::<MaterialConsumerIndex>().is_none() {
        world.init_resource::<MaterialConsumerIndex>();
    }
    let memo_key = binding
        .as_ref()
        .map(|path| (path.clone(), ctx.time.map(f64::to_bits)));
    let memoized = memo_key.as_ref().and_then(|key| {
        world
            .get_resource::<InitialMaterialMemo>()?
            .entries
            .get(key)
            .cloned()
    });
    let (dependencies, network_resolved) = if let Some(read) = &memoized {
        (read.dependencies.clone(), true)
    } else if let Some(path) = &binding_path {
        match material_network_dependencies(ctx.stage, path) {
            Ok(dependencies) => (dependencies, true),
            Err(_) => (vec![path.as_str().to_owned()], false),
        }
    } else {
        (Vec::new(), false)
    };
    world.resource_mut::<MaterialConsumerIndex>().update(
        ctx.prim_str(),
        binding.as_deref(),
        &dependencies,
        entity,
    );

    let material = binding_path.as_ref().and_then(|binding_path| {
        if let Some(handle) = world
            .get_resource_mut::<ProjectionSeed>()
            .and_then(|mut seeds| seeds.take_authoritative_material(ctx.prim_str()))
            && world
                .resource::<Assets<StandardMaterial>>()
                .contains(&handle)
        {
            world
                .resource_mut::<MaterialProjectionProvenance>()
                .mark_authored(ctx.prim_str());
            return Some(handle);
        }
        let descriptor = if let Some(read) = memoized {
            read.descriptor
        } else {
            super::record_descriptor_read(world);
            let descriptor = read_preview_material(ctx.stage, binding_path)
                .ok()
                .flatten()?;
            if network_resolved
                && let Some(key) = memo_key
                && let Some(mut memo) = world.get_resource_mut::<InitialMaterialMemo>()
                && memo.entries.len() < 256
                && static_untextured_material(ctx, &descriptor, &dependencies)
            {
                memo.entries.insert(
                    key,
                    InitialMaterialRead {
                        descriptor: descriptor.clone(),
                        dependencies,
                    },
                );
            }
            descriptor
        };
        let handle = intern_material(world, binding_path.as_str(), &descriptor);
        if handle.is_some() {
            world
                .resource_mut::<MaterialProjectionProvenance>()
                .mark_authored(ctx.prim_str());
        }
        handle
    });
    if material.is_none() {
        world
            .resource_mut::<MaterialProjectionProvenance>()
            .mark_fallback(ctx.prim_str());
    }
    let Some(handle) = material.or_else(|| {
        world
            .get::<Mesh3d>(entity)
            .is_some()
            .then(|| crate::route::fallback_material(world))
    }) else {
        return;
    };
    if world
        .get::<crate::extended_skin::ExtendedSkinMesh>(entity)
        .is_some()
    {
        if let Some(base) = world
            .resource::<Assets<StandardMaterial>>()
            .get(&handle)
            .cloned()
        {
            if crate::extended_skin::set_extended_material(world, entity, base) {
                mark_render_projection_dirty(world, entity);
                return;
            }
        }
    }
    let changed = if let Some(mut mat) = world.get_mut::<MeshMaterial3d<StandardMaterial>>(entity) {
        if mat.0 == handle {
            false
        } else {
            mat.0 = handle;
            true
        }
    } else if let Ok(mut e) = world.get_entity_mut(entity) {
        e.insert(MeshMaterial3d(handle));
        true
    } else {
        false
    };
    if changed {
        mark_render_projection_dirty(world, entity);
    }
}

fn static_untextured_material(
    ctx: &RouteCtx,
    descriptor: &ReadPreviewMaterial,
    dependencies: &[String],
) -> bool {
    descriptor.uv_transform.is_none()
        && [
            &descriptor.diffuse_texture,
            &descriptor.normal_texture,
            &descriptor.roughness_texture,
            &descriptor.metallic_texture,
            &descriptor.opacity_texture,
            &descriptor.emissive_texture,
            &descriptor.occlusion_texture,
        ]
        .into_iter()
        .all(Option::is_none)
        && dependencies.len() <= 64
        && dependencies.iter().all(|path| {
            openusd::sdf::path(path)
                .ok()
                .and_then(|path| ctx.stage.prim(path).ok())
                .and_then(|prim| prim.attributes().ok())
                .is_some_and(|attributes| {
                    attributes.into_iter().all(|attribute| {
                        attribute
                            .time_sample_times()
                            .is_ok_and(|samples| samples.is_empty())
                    })
                })
        })
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
        if world.get_resource::<Assets<StandardMaterial>>().is_none()
            || world.get::<super::super::geom::DeferredSourceMesh>(entity).is_some()
        {
            return;
        }
        apply_bound_material(ctx, world, entity);
    }

    fn patch(&self, ctx: &RouteCtx, world: &mut World, entity: Entity, changed: &[&str]) {
        if !material_patch_relevant(ctx, changed)
            || world.get::<super::super::geom::DeferredSourceMesh>(entity).is_some()
        {
            return;
        }
        apply_bound_material(ctx, world, entity);
        if is_material_network_prim(ctx) {
            reproject_consumers(ctx, world);
        }
    }
}
