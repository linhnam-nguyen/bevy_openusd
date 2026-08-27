//! Points route (SCHEMA_INTEGRATION Phase C): `UsdGeomPoints` → a Bevy
//! `PointList` mesh (one vertex per point). Read through the geom `Points` /
//! `PointBased` schema.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;

use openusd::schemas::geom::{PointBased, Points};
use openusd::sdf::Value;

use super::{PrimRoute, RouteCtx, track_mesh_projection};

/// Maps `UsdGeomPoints` to a point-cloud mesh.
pub struct PointsRoute;

fn positions(ctx: &RouteCtx) -> Option<Vec<[f32; 3]>> {
    let points = Points::get(ctx.stage, ctx.path.clone()).ok()??;
    match points.points_attr().get::<Value>() {
        Ok(Some(Value::Vec3fVec(v))) => Some(v.iter().map(|p| [p.x, p.y, p.z]).collect()),
        Ok(Some(Value::Vec3dVec(v))) => Some(
            v.iter()
                .map(|p| [p.x as f32, p.y as f32, p.z as f32])
                .collect(),
        ),
        _ => None,
    }
}

impl PrimRoute for PointsRoute {
    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.as_deref() == Some("Points")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        if world.get_resource::<Assets<Mesh>>().is_none()
            || world.get_resource::<Assets<StandardMaterial>>().is_none()
        {
            return;
        }
        let Some(pos) = positions(ctx) else {
            return;
        };
        if pos.is_empty() {
            return;
        }
        let mut mesh = Mesh::new(PrimitiveTopology::PointList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        let mesh_handle = super::cache::intern_mesh(world, mesh);
        let material = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let projected = if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert((Mesh3d(mesh_handle), MeshMaterial3d(material)));
            true
        } else {
            false
        };
        if projected {
            let mesh = world.get::<Mesh3d>(entity).map(|mesh| mesh.0.clone());
            if let Some(mesh) = mesh {
                track_mesh_projection(world, entity, &mesh);
            }
        }
    }
}
