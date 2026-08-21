//! Geometry routes: `visibility` → Bevy [`Visibility`], and mesh prims →
//! [`Mesh3d`] + a placeholder [`MeshMaterial3d`]. Real material binding (from
//! `read::shade`) layers on as its own route later (PLAN P4).

use bevy::prelude::*;
use std::time::Instant;

use super::cache::{intern_mesh, intern_mesh_profiled};
use super::profile::{
    GeometryProfile, GeometryProfileRecord, GeometrySubdivisionClass, REASON_CACHE_MISS,
    REASON_DISPLAY_COLOR, REASON_DISPLAY_OPACITY, REASON_EXPANDED_PRIMVARS,
    REASON_GENERATED_NORMALS, REASON_HIGH_VERTEX_EXPANSION, REASON_SUBDIVISION, hash_prim_path,
};
use super::{DisplayPurposes, PrimRoute, RouteCtx};
use crate::read::geom::{VisibilityState, read_effective_purpose, read_mesh, read_visibility};

/// The prim's effective (inherited) USD `purpose`: `"default"`, `"render"`,
/// `"proxy"`, or `"guide"`. Carried so gameplay/UI can query or re-filter it.
#[derive(Component, Debug, Clone)]
pub struct UsdPurpose(pub String);

/// Maps `visibility` (+ `purpose`) → [`Visibility`]. Applies to every prim
/// (imageable or not); an unauthored `visibility` reads as inherited/visible.
///
/// A prim is hidden if it is authored `invisible` **or** its effective purpose
/// isn't in the world's [`DisplayPurposes`] (PLAN Phase A) — so `guide`
/// annotations and, by default, the `render` twin of a `proxy`/`render` pair
/// don't draw. `purpose` is inherited down namespace, so this resolves the
/// effective purpose from the nearest ancestor with an authored opinion.
pub struct VisibilityRoute;

/// Combined visibility + effective purpose for `entity`'s prim, honoring the
/// world's [`DisplayPurposes`] (defaults when the resource is absent).
fn resolve(ctx: &RouteCtx, world: &World) -> (Visibility, String) {
    let purpose =
        read_effective_purpose(ctx.stage, ctx.path).unwrap_or_else(|_| "default".to_string());
    let purposes = world
        .get_resource::<DisplayPurposes>()
        .copied()
        .unwrap_or_default();
    let invisible = matches!(
        read_visibility(ctx.stage, ctx.path),
        Ok(VisibilityState::Invisible)
    );
    let hidden = invisible || !purposes.shows(&purpose);
    let vis = if hidden {
        Visibility::Hidden
    } else {
        Visibility::default()
    };
    (vis, purpose)
}

fn apply(ctx: &RouteCtx, world: &mut World, entity: Entity) {
    let (vis, purpose) = resolve(ctx, world);
    if let Ok(mut e) = world.get_entity_mut(entity) {
        e.insert((vis, UsdPurpose(purpose)));
    }
}

impl PrimRoute for VisibilityRoute {
    fn matches(&self, _ctx: &RouteCtx) -> bool {
        true
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        apply(ctx, world, entity);
    }

    fn patch(&self, ctx: &RouteCtx, world: &mut World, entity: Entity, changed: &[&str]) {
        let touches =
            changed.is_empty() || changed.contains(&"visibility") || changed.contains(&"purpose");
        if !touches {
            return;
        }
        apply(ctx, world, entity);
    }
}

/// Bakes a UsdGeomMesh's points/topology into a Bevy [`Mesh`] and attaches
/// [`Mesh3d`] + a default [`StandardMaterial`]. No-op (with a warning) when the
/// render `Assets` are absent (headless) — the prim still projects, it just
/// carries no renderable geometry.
pub struct MeshRoute;

impl MeshRoute {
    /// Bake + attach; returns whether a `Mesh3d` was inserted.
    fn attach(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) -> bool {
        let profile_enabled = world
            .get_resource::<GeometryProfile>()
            .is_some_and(|profile| profile.enabled);
        let read_start = profile_enabled.then(Instant::now);
        let read_result = read_mesh(ctx.stage, ctx.path);
        let read_mesh_ms = read_start
            .map(|started| started.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or_default();
        let Ok(Some(read)) = read_result else {
            return false;
        };
        if world.get_resource::<Assets<Mesh>>().is_none()
            || world.get_resource::<Assets<StandardMaterial>>().is_none()
        {
            bevy::log::warn!(
                target: "usd_bevy::route::geom",
                "{}: has a mesh but render Assets are absent — not attached",
                ctx.prim_str()
            );
            return false;
        }
        bevy::log::trace!(
            target: "usd_bevy::route::geom",
            "{}: mesh {} points -> Mesh3d",
            ctx.prim_str(),
            read.points.len()
        );
        let (mesh, build_metrics) = if profile_enabled {
            crate::mesh::mesh_from_usd_profiled(&read)
        } else {
            (crate::mesh::mesh_from_usd(&read), Default::default())
        };
        let (mesh_handle, intern_metrics) = if profile_enabled {
            intern_mesh_profiled(world, mesh)
        } else {
            (intern_mesh(world, mesh), Default::default())
        };
        if profile_enabled {
            let mut reason_flags = 0;
            if build_metrics.expanded_primvars > 0 {
                reason_flags |= REASON_EXPANDED_PRIMVARS;
            }
            if build_metrics.generated_normals {
                reason_flags |= REASON_GENERATED_NORMALS;
            }
            if build_metrics.subdivision != GeometrySubdivisionClass::None {
                reason_flags |= REASON_SUBDIVISION;
            }
            if build_metrics.display_color {
                reason_flags |= REASON_DISPLAY_COLOR;
            }
            if build_metrics.display_opacity {
                reason_flags |= REASON_DISPLAY_OPACITY;
            }
            if !intern_metrics.cache_hit {
                reason_flags |= REASON_CACHE_MISS;
            }
            if build_metrics.vertex_source_ratio > 1.0 {
                reason_flags |= REASON_HIGH_VERTEX_EXPANSION;
            }
            world
                .resource_mut::<GeometryProfile>()
                .record(GeometryProfileRecord {
                    prim_path_hash: hash_prim_path(ctx.path.as_str()),
                    read_mesh_ms,
                    mesh_from_usd_ms: build_metrics.mesh_from_usd_ms,
                    topology_triangulation_ms: build_metrics.topology_triangulation_ms,
                    primvar_expansion_ms: build_metrics.primvar_expansion_ms,
                    normal_generation_ms: build_metrics.normal_generation_ms,
                    bevy_mesh_allocation_ms: intern_metrics.allocation_ms,
                    mesh_signature_ms: intern_metrics.signature_ms,
                    mesh_intern_ms: intern_metrics.total_ms,
                    source_points: build_metrics.source_points,
                    source_faces: build_metrics.source_faces,
                    source_face_corners: build_metrics.source_face_corners,
                    output_vertices: build_metrics.output_vertices,
                    output_indices: build_metrics.output_indices,
                    output_triangles: build_metrics.output_triangles,
                    authored_normals: build_metrics.authored_normals,
                    generated_normals: build_metrics.generated_normals,
                    expanded_vertices: build_metrics.expanded_vertices,
                    cache_hit: intern_metrics.cache_hit,
                    uv_interpolation: build_metrics.uv_interpolation,
                    indexed_primvars: build_metrics.indexed_primvars,
                    expanded_primvars: build_metrics.expanded_primvars,
                    display_color: build_metrics.display_color,
                    display_opacity: build_metrics.display_opacity,
                    topology_class: build_metrics.topology_class,
                    subdivision: build_metrics.subdivision,
                    vertex_source_ratio: build_metrics.vertex_source_ratio,
                    reason_flags,
                });
        }
        let material = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        if let Ok(mut e) = world.get_entity_mut(entity) {
            if let Some([min, max]) = read.extent {
                e.insert(UsdLocalExtent { min, max });
            } else {
                e.remove::<UsdLocalExtent>();
            }
            e.insert((Mesh3d(mesh_handle), MeshMaterial3d(material)));
            return true;
        }
        false
    }
}

impl PrimRoute for MeshRoute {
    fn matches(&self, ctx: &RouteCtx) -> bool {
        // Fast path on typeName; fall back to probing for `points` so meshes
        // authored without an explicit type (rare, but valid) still route.
        matches!(ctx.type_name.as_deref(), Some("Mesh"))
            || read_mesh(ctx.stage, ctx.path).ok().flatten().is_some()
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        self.attach(ctx, world, entity);
    }

    // patch falls back to project (rebuild the mesh). Mesh topology changes
    // arrive via `resynced` in practice, so this is rarely hit on changed_info.
}

/// Authored `UsdGeomBoundable.extent` — `[min, max]` corners in the prim's local space.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
#[reflect(Component, Default)]
pub struct UsdLocalExtent {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// Authored `UsdModelAPI.kind`.
#[derive(Component, Reflect, Debug, Clone, Default, PartialEq, Eq)]
#[reflect(Component, Default)]
pub struct UsdKind {
    pub kind: String,
}

/// Authored `ui:displayName`.
#[derive(Component, Reflect, Debug, Clone, Default, PartialEq, Eq)]
#[reflect(Component, Default)]
pub struct UsdDisplayName(pub String);
