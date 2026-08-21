//! Shapes route (SCHEMA_INTEGRATION Phase B): USD geom shape prims
//! (`Cube`/`Sphere`/`Cylinder`/`Capsule`/`Cone`/`Plane`) → Bevy primitive
//! meshes. Reads through openusd's `geom` shape schemas.
//!
//! USD's `Cylinder`/`Cone`/`Capsule`/`Plane` carry an `axis` (default `Z`);
//! Bevy primitives are `Y`-aligned, so the generated mesh is rotated to match.
//! Registered before the material route, which then binds a real material.

use bevy::prelude::*;
use std::f32::consts::FRAC_PI_2;

use openusd::schemas::geom::{Capsule, Cone, Cube, Cylinder, Plane, Sphere};
use openusd::sdf::Value;

use super::{PrimRoute, RouteCtx};

/// Maps USD geometric shapes to Bevy primitive meshes.
pub struct ShapesRoute;

fn f32_attr(attr: openusd::usd::Attribute, default: f32) -> f32 {
    match attr.get::<Value>() {
        Ok(Some(Value::Double(d))) => d as f32,
        Ok(Some(Value::Float(f))) => f,
        _ => default,
    }
}

/// Rotation taking a `Y`-aligned Bevy primitive onto the USD `axis` token.
fn axis_rotation(attr: openusd::usd::Attribute) -> Quat {
    let axis = match attr.get::<Value>() {
        Ok(Some(Value::Token(t))) => t.as_str().to_string(),
        _ => "Z".to_string(), // USD default axis
    };
    match axis.as_str() {
        "X" => Quat::from_rotation_z(-FRAC_PI_2), // +Y → +X
        "Z" => Quat::from_rotation_x(FRAC_PI_2),  // +Y → +Z
        _ => Quat::IDENTITY,                      // "Y"
    }
}

fn shape_mesh(ctx: &RouteCtx) -> Option<Mesh> {
    let stage = ctx.stage;
    let p = ctx.path.clone();
    match ctx.type_name.as_deref()? {
        "Cube" => {
            let cube = Cube::get(stage, p).ok()??;
            let size = f32_attr(cube.size_attr(), 2.0);
            Some(Mesh::from(Cuboid::from_length(size)))
        }
        "Sphere" => {
            let sphere = Sphere::get(stage, p).ok()??;
            let r = f32_attr(sphere.radius_attr(), 1.0);
            Some(Mesh::from(bevy::math::primitives::Sphere::new(r)))
        }
        "Cylinder" => {
            let cyl = Cylinder::get(stage, p).ok()??;
            let r = f32_attr(cyl.radius_attr(), 1.0);
            let h = f32_attr(cyl.height_attr(), 2.0);
            let mesh = Mesh::from(bevy::math::primitives::Cylinder::new(r, h));
            Some(mesh.rotated_by(axis_rotation(cyl.axis_attr())))
        }
        "Capsule" => {
            let cap = Capsule::get(stage, p).ok()??;
            let r = f32_attr(cap.radius_attr(), 0.5);
            let h = f32_attr(cap.height_attr(), 1.0);
            let mesh = Mesh::from(Capsule3d::new(r, h));
            Some(mesh.rotated_by(axis_rotation(cap.axis_attr())))
        }
        "Cone" => {
            let cone = Cone::get(stage, p).ok()??;
            let r = f32_attr(cone.radius_attr(), 1.0);
            let h = f32_attr(cone.height_attr(), 2.0);
            let mesh = Mesh::from(bevy::math::primitives::Cone::new(r, h));
            Some(mesh.rotated_by(axis_rotation(cone.axis_attr())))
        }
        "Plane" => {
            let plane = Plane::get(stage, p).ok()??;
            let w = f32_attr(plane.width_attr(), 1.0);
            let l = f32_attr(plane.length_attr(), 1.0);
            let mesh = Mesh::from(Rectangle::new(w, l));
            // Rectangle lies in the XY plane (normal +Z); USD plane's normal is
            // its `axis`. Rotate XY→ the axis plane.
            Some(mesh.rotated_by(axis_rotation(plane.axis_attr())))
        }
        _ => None,
    }
}

fn shape_patch_relevant(changed: &[&str]) -> bool {
    changed.is_empty()
        || changed.iter().any(|property| {
            matches!(
                *property,
                "size" | "radius" | "height" | "width" | "length" | "axis"
            )
        })
}

impl PrimRoute for ShapesRoute {
    fn matches(&self, ctx: &RouteCtx) -> bool {
        matches!(
            ctx.type_name.as_deref(),
            Some("Cube" | "Sphere" | "Cylinder" | "Capsule" | "Cone" | "Plane")
        )
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        if world.get_resource::<Assets<Mesh>>().is_none()
            || world.get_resource::<Assets<StandardMaterial>>().is_none()
        {
            return;
        }
        let Some(mesh) = shape_mesh(ctx) else {
            return;
        };
        let mesh_handle = super::cache::intern_mesh(world, mesh);
        let material = world
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .map(|material| material.0.clone())
            .unwrap_or_else(|| {
                world
                    .resource_mut::<Assets<StandardMaterial>>()
                    .add(StandardMaterial::default())
            });
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert((Mesh3d(mesh_handle), MeshMaterial3d(material)));
        }
    }

    fn patch(&self, ctx: &RouteCtx, world: &mut World, entity: Entity, changed: &[&str]) {
        if shape_patch_relevant(changed) {
            self.project(ctx, world, entity);
        }
    }
}
