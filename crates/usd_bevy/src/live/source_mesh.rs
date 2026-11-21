//! Owned input and revision-checked publication for canonical mesh preparation.

use bevy::prelude::*;

use super::{LiveRevision, LiveStage, ProgressiveProjectionState, VisualContextId};
use crate::read::geom::{ReadMesh, read_mesh};
use crate::route::geom::{DeferredSourceMesh, MeshRoute, deferred_mesh_supported};
use crate::route::{PrimRoute, RouteCtx, StageTime, cache::intern_mesh, material::MaterialRoute};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceMeshIdentity {
    pub entity: Entity,
    context: VisualContextId,
    authoring: LiveRevision,
    projection_generation: u64,
    time: Option<u64>,
}

impl SourceMeshIdentity {
    pub fn capture(world: &World, entity: Entity) -> Option<Self> {
        world.get::<DeferredSourceMesh>(entity)?;
        let live = world.get_non_send::<LiveStage>()?;
        let projection = world.get_resource::<ProgressiveProjectionState>()?;
        if projection.session_id() != Some(live.session_id()) || live.has_changes() {
            return None;
        }
        Some(Self {
            entity,
            context: live.visual_context_id(),
            authoring: live.authoring_generation(),
            projection_generation: projection.generation(),
            time: world
                .get_resource::<StageTime>()
                .map(|clock| clock.current.to_bits()),
        })
    }
}

pub struct SourceMeshInput {
    pub identity: SourceMeshIdentity,
    read: ReadMesh,
}

impl SourceMeshInput {
    pub fn capture(world: &World, identity: SourceMeshIdentity) -> Option<Self> {
        if SourceMeshIdentity::capture(world, identity.entity) != Some(identity) {
            return None;
        }
        let live = world.get_non_send::<LiveStage>()?;
        let stage = live.projection_stage();
        let path =
            openusd::sdf::path(&world.get::<crate::UsdPrimRef>(identity.entity)?.path).ok()?;
        let ctx = RouteCtx::at(&stage, &path, identity.time.map(f64::from_bits));
        if !deferred_mesh_supported(&ctx) {
            return None;
        }
        let read = read_mesh(&stage, &path).ok()??;
        if read.subdivision_scheme.is_subdivision() || !read.subsets.is_empty() {
            return None;
        }
        Some(Self { identity, read })
    }

    pub fn estimated_bytes(&self) -> u64 {
        let read = &self.read;
        fn bytes<T>(values: &Vec<T>) -> u64 {
            (values.capacity() as u64).saturating_mul(std::mem::size_of::<T>() as u64)
        }
        fn primvar_bytes<T>(primvar: &Option<crate::read::geom::MeshPrimvar<T>>) -> u64 {
            primvar.as_ref().map_or(0, |value| {
                bytes(&value.values).saturating_add(bytes(&value.indices))
            })
        }
        [
            bytes(&read.points),
            bytes(&read.face_vertex_counts),
            bytes(&read.face_vertex_indices),
            primvar_bytes(&read.normals),
            primvar_bytes(&read.uvs),
            primvar_bytes(&read.display_color),
            primvar_bytes(&read.display_opacity),
            128u64.saturating_mul(read.points.len().max(read.face_vertex_indices.len()) as u64),
        ]
        .into_iter()
        .fold(0, u64::saturating_add)
    }

    pub fn prepare(self) -> Mesh {
        crate::mesh::mesh_from_usd(&self.read)
    }
}

/// Publish to the canonical logical prim. An absent prepared mesh uses the stock route.
pub fn publish_source_mesh(world: &mut World, identity: SourceMeshIdentity, mesh: Option<Mesh>) {
    if SourceMeshIdentity::capture(world, identity.entity) != Some(identity) {
        return;
    }
    let entity = identity.entity;
    let stage = world
        .get_non_send::<LiveStage>()
        .expect("current stage")
        .projection_stage();
    let Some(path) = world
        .get::<crate::UsdPrimRef>(entity)
        .and_then(|prim| openusd::sdf::path(&prim.path).ok())
    else {
        return;
    };
    world.entity_mut(entity).remove::<DeferredSourceMesh>();
    let ctx = RouteCtx::at(&stage, &path, identity.time.map(f64::from_bits));
    if let Some(mesh) = mesh {
        let handle = intern_mesh(world, mesh);
        let fallback = crate::route::fallback_material(world);
        world
            .entity_mut(entity)
            .insert((Mesh3d(handle.clone()), MeshMaterial3d(fallback)));
        crate::route::track_mesh_projection(world, entity, &handle);
    } else {
        MeshRoute.attach(&ctx, world, entity);
    }
    MaterialRoute.project(&ctx, world, entity);
}
