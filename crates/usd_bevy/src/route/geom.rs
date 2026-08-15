//! Geometry routes: `visibility` → Bevy [`Visibility`], and mesh prims →
//! [`Mesh3d`] + a placeholder [`MeshMaterial3d`]. Real material binding (from
//! `read::shade`) layers on as its own route later (PLAN P4).

use bevy::prelude::*;

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
        let Ok(Some(read)) = read_mesh(ctx.stage, ctx.path) else {
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
        let mesh = crate::mesh::mesh_from_usd(&read);
        let mesh_handle = super::cache::intern_mesh(world, mesh);
        let material = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        if let Ok(mut e) = world.get_entity_mut(entity) {
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
