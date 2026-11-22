//! Shared prototype assets and occurrence-local parts.

use std::sync::Arc;
use std::time::Instant;

use bevy::camera::primitives::{Aabb, MeshAabb};
use bevy::math::Affine3A;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use openusd::sdf::Path;

use crate::read::geom::{VisibilityState, read_mesh, read_visibility_at};
use crate::read::shade::{
    material_network_dependencies, read_material_binding, read_preview_material,
};
use crate::read::xform::read_local_xform_at;
use crate::route::{
    RouteCtx, fallback_material, mark_render_projection_dirty, material::intern_material,
    profile::GeometryProfile, remove_mesh_projection_consumer, track_mesh_projection,
    xform::UsdFullLocalAffine,
};

/// Source path of a generated renderable within a PointInstancer occurrence.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct UsdPrototypePart(pub Arc<str>);

struct Part {
    path: UsdPrototypePart,
    affine: Affine3A,
    visible: bool,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

#[derive(Default)]
pub(super) struct Prototype {
    parts: Vec<Part>,
    pub(super) dependencies: Vec<String>,
    pub(super) bounds: Option<super::UsdOccurrenceBounds>,
}

impl Prototype {
    pub(super) fn gpu_meshes(
        &self,
        world: &World,
    ) -> Option<Vec<crate::route::gpu_instancing::PreparedGpuMesh>> {
        use crate::route::gpu_instancing::PreparedGpuMesh;
        use bevy::mesh::PrimitiveTopology;
        use bevy::render::render_resource::Face;

        let meshes = world.resource::<Assets<Mesh>>();
        let materials = world.resource::<Assets<StandardMaterial>>();
        self.parts
            .iter()
            .filter(|part| part.visible)
            .map(|part| {
                let mesh = meshes.get(&part.mesh)?;
                let material = materials.get(&part.material)?;
                let affine = Mat4::from(part.affine);
                let supported = mesh.primitive_topology() == PrimitiveTopology::TriangleList
                    && mesh.indices().is_some()
                    && mesh.attribute(Mesh::ATTRIBUTE_POSITION).is_some()
                    && affine.determinant().is_finite()
                    && affine.determinant() > 1e-12
                    && material.alpha_mode == AlphaMode::Opaque
                    && !material.double_sided
                    && material.cull_mode == Some(Face::Back)
                    && !material.unlit
                    && material.depth_bias == 0.0
                    && material.diffuse_transmission == 0.0
                    && material.specular_transmission == 0.0
                    && material.clearcoat == 0.0
                    && material.anisotropy_strength == 0.0
                    && [
                        &material.base_color_texture,
                        &material.emissive_texture,
                        &material.metallic_roughness_texture,
                        &material.normal_map_texture,
                        &material.occlusion_texture,
                    ]
                    .iter()
                    .all(|texture| texture.is_none());
                if !supported {
                    return None;
                }
                PreparedGpuMesh::new(part.mesh.clone(), mesh, part.material.clone(), affine)
            })
            .collect()
    }
}

pub(super) fn prepare(ctx: &RouteCtx, world: &mut World, root: &Path) -> Prototype {
    let mut prototype = Prototype::default();
    if let Err(error) = collect(
        ctx,
        world,
        root,
        Affine3A::IDENTITY,
        true,
        0,
        &mut prototype,
    ) {
        bevy::log::warn!(
            target: "usd_bevy::route::instancer",
            "{}: failed to prepare prototype {root}: {error:#}",
            ctx.prim_str()
        );
        return Prototype::default();
    }
    prototype
}

// The upstream prototype hierarchy traversal is adapted to compose full
// affine matrices in the occurrence's coordinate space.
fn collect(
    ctx: &RouteCtx,
    world: &mut World,
    path: &Path,
    parent_affine: Affine3A,
    parent_visible: bool,
    depth: usize,
    prototype: &mut Prototype,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        depth < 256 && prototype.parts.len() < 4096,
        "prototype exceeds traversal bounds"
    );
    let prim = ctx.stage.prim(path.clone())?;
    if !prim.is_active()? || !prim.is_defined()? || prim.is_abstract()? {
        return Ok(());
    }
    if prim.is_instance_proxy()? {
        prototype.dependencies.extend(
            crate::live::native_proxy_sources(ctx.stage, path)
                .into_iter()
                .map(|path| path.to_string()),
        );
    }
    let kind = prim.type_name()?;
    if matches!(
        kind.as_deref(),
        Some("Material" | "Shader" | "GeomSubset" | "PointInstancer")
    ) {
        return Ok(());
    }
    let affine = match read_local_xform_at(ctx.stage, path, ctx.time)? {
        Some(local) if local.resets_xform_stack => Affine3A::from_mat4(local.matrix),
        Some(local) => parent_affine * Affine3A::from_mat4(local.matrix),
        None => parent_affine,
    };
    let visible = parent_visible
        && read_visibility_at(ctx.stage, path, ctx.time)? != VisibilityState::Invisible;
    if kind.as_deref() == Some("Mesh") {
        let read_start = world
            .get_resource::<GeometryProfile>()
            .is_some_and(|profile| profile.enabled)
            .then(Instant::now);
        if let Some(mesh) = read_mesh(ctx.stage, path)? {
            let read_ms = read_start.map_or(0.0, |start| start.elapsed().as_secs_f64() * 1000.0);
            let mesh = super::bake_mesh(world, path, mesh, read_ms);
            if visible
                && let Some(bounds) = world
                    .resource::<Assets<Mesh>>()
                    .get(&mesh)
                    .and_then(MeshAabb::get_aabb)
            {
                let center = Vec3::from(bounds.center);
                let half = Vec3::from(bounds.half_extents);
                let occurrence_bounds =
                    prototype.bounds.get_or_insert(super::UsdOccurrenceBounds {
                        min: Vec3::splat(f32::INFINITY),
                        max: Vec3::splat(f32::NEG_INFINITY),
                    });
                for axis in 0..8 {
                    let sign = Vec3::new(
                        if axis & 1 == 0 { -1.0 } else { 1.0 },
                        if axis & 2 == 0 { -1.0 } else { 1.0 },
                        if axis & 4 == 0 { -1.0 } else { 1.0 },
                    );
                    let corner = affine.transform_point3(center + sign * half);
                    occurrence_bounds.min = occurrence_bounds.min.min(corner);
                    occurrence_bounds.max = occurrence_bounds.max.max(corner);
                }
            }
            let binding = read_material_binding(ctx.stage, path)?;
            let material = binding
                .as_ref()
                .and_then(|binding| {
                    prototype.dependencies.push(binding.to_string());
                    if let Ok(dependencies) = material_network_dependencies(ctx.stage, binding) {
                        prototype.dependencies.extend(dependencies);
                    }
                    let description = read_preview_material(ctx.stage, binding).ok().flatten()?;
                    intern_material(world, binding.as_str(), &description)
                })
                .unwrap_or_else(|| fallback_material(world));
            prototype.parts.push(Part {
                path: UsdPrototypePart(Arc::from(path.as_str())),
                affine,
                visible,
                mesh,
                material,
            });
        }
    }
    for child in prim.children()? {
        collect(
            ctx,
            world,
            child.path(),
            affine,
            visible,
            depth + 1,
            prototype,
        )?;
    }
    Ok(())
}

pub(super) fn reconcile_parts(
    world: &mut World,
    occurrence: Entity,
    prototype: Option<&Prototype>,
    masked: bool,
) {
    let parts = prototype.map_or(&[][..], |prototype| prototype.parts.as_slice());
    if let [part] = parts
        && part.affine == Affine3A::IDENTITY
    {
        remove_part_children(world, occurrence);
        apply_part(world, occurrence, part, masked);
        return;
    }

    world.entity_mut(occurrence).remove::<UsdPrototypePart>();
    remove_renderable(world, occurrence);
    let children = part_children(world, occurrence);
    let mut previous: HashMap<_, _> = children.into_iter().collect();
    for part in parts {
        let entity = previous.remove(&part.path.0).unwrap_or_else(|| {
            world
                .spawn((
                    Transform::IDENTITY,
                    Visibility::Inherited,
                    ChildOf(occurrence),
                ))
                .id()
        });
        let local = UsdFullLocalAffine(part.affine);
        if world.get::<UsdFullLocalAffine>(entity) != Some(&local) {
            world.entity_mut(entity).insert(local);
        }
        let visibility = if part.visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if world.get::<Visibility>(entity) != Some(&visibility) {
            world.entity_mut(entity).insert(visibility);
        }
        apply_part(world, entity, part, masked);
    }
    for entity in previous.into_values() {
        remove_mesh_projection_consumer(world, entity);
        world.despawn(entity);
    }
}

fn part_children(world: &World, occurrence: Entity) -> Vec<(Arc<str>, Entity)> {
    world
        .get::<Children>(occurrence)
        .into_iter()
        .flat_map(|children| children.iter())
        .filter_map(|entity| {
            let path = world.get::<UsdPrototypePart>(entity)?.0.clone();
            Some((path, entity))
        })
        .collect()
}

fn remove_part_children(world: &mut World, occurrence: Entity) {
    for (_, entity) in part_children(world, occurrence) {
        remove_mesh_projection_consumer(world, entity);
        world.despawn(entity);
    }
}

fn apply_part(world: &mut World, entity: Entity, part: &Part, masked: bool) {
    if world.get::<UsdPrototypePart>(entity) != Some(&part.path) {
        world.entity_mut(entity).insert(part.path.clone());
    }
    if masked || !part.visible {
        remove_renderable(world, entity);
        return;
    }
    if world
        .get::<Mesh3d>(entity)
        .is_none_or(|mesh| mesh.0 != part.mesh)
    {
        world
            .entity_mut(entity)
            .remove::<Aabb>()
            .insert(Mesh3d(part.mesh.clone()));
        track_mesh_projection(world, entity, &part.mesh);
    }
    if world
        .get::<MeshMaterial3d<StandardMaterial>>(entity)
        .is_none_or(|material| material.0 != part.material)
    {
        world
            .entity_mut(entity)
            .insert(MeshMaterial3d(part.material.clone()));
        mark_render_projection_dirty(world, entity);
    }
}

fn remove_renderable(world: &mut World, entity: Entity) {
    if world.get::<Mesh3d>(entity).is_some() {
        world
            .entity_mut(entity)
            .remove::<(Mesh3d, MeshMaterial3d<StandardMaterial>, Aabb)>();
        remove_mesh_projection_consumer(world, entity);
    }
}
