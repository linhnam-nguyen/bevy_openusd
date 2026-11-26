//! Geometry routes: `visibility` → Bevy [`Visibility`], and mesh prims →
//! [`Mesh3d`] + a placeholder [`MeshMaterial3d`]. Real material binding (from
//! `read::shade`) layers on as its own route later (PLAN P4).

use bevy::prelude::*;
use std::time::Instant;

#[path = "hierarchy.rs"]
mod hierarchy;

pub(crate) fn prepare_hierarchy_metadata(stage: &openusd::usd::Stage, world: &mut World) {
    hierarchy::prepare_metadata_index(stage, world);
}

/// Authoritative presentation metadata for a composed prim. This is not a
/// semantic key and does not change the canonical prim identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredHierarchyMetadata {
    /// Authored display name before product-specific overrides.
    pub display_name: Option<String>,
    /// Manifest-backed hierarchy target, when authored on the composed prim.
    pub target: Option<crate::UsdHierarchyTarget>,
}

/// Read the same display-name and target metadata used by live projection,
/// without projecting geometry or opening another Stage. A read failure is
/// distinct from a successful read of a prim with no authored name.
pub fn read_authored_hierarchy_metadata(
    stage: &openusd::usd::Stage,
    prim_path: &str,
) -> anyhow::Result<AuthoredHierarchyMetadata> {
    hierarchy::read_authored_hierarchy_metadata(stage, prim_path)
}

/// Invalidate Stage-owned hierarchy metadata once at the live change-batch
/// boundary. The next prim projection rebuilds the shared index once.
pub(crate) fn note_hierarchy_metadata_revision(world: &mut World, revision: u64) {
    hierarchy::note_metadata_revision(world, revision);
}

use super::ProjectionSeed;
use super::cache::{
    ProjectionCache, intern_mesh, intern_mesh_profiled, lookup_source_mesh, remember_source_mesh,
};
use super::cache_key::source_mesh_key;
use super::profile::{GeometryProfile, hash_prim_path, record_mesh_sample};
use super::{DisplayPurposes, PrimRoute, RouteCtx, track_mesh_projection};
use crate::live::PerformanceCounters;
use crate::read::geom::{
    VisibilityState, read_effective_purpose, read_mesh, read_mesh_extent, read_visibility_at,
};

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
        read_visibility_at(ctx.stage, ctx.path, ctx.time),
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
    hierarchy::apply_metadata(ctx, world, entity);
}

impl PrimRoute for VisibilityRoute {
    fn matches(&self, _ctx: &RouteCtx) -> bool {
        true
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        apply(ctx, world, entity);
    }

    fn patch(&self, ctx: &RouteCtx, world: &mut World, entity: Entity, changed: &[&str]) {
        if changed.is_empty() || changed.contains(&"visibility") || changed.contains(&"purpose") {
            let (vis, purpose) = resolve(ctx, world);
            if let Ok(mut e) = world.get_entity_mut(entity) {
                e.insert((vis, UsdPurpose(purpose)));
            }
        }
        if changed.is_empty()
            || changed.contains(&"ui:displayName")
            || changed.contains(&"customData")
            || changed
                .iter()
                .any(|property| property.starts_with("customData:"))
        {
            hierarchy::apply_metadata(ctx, world, entity);
        }
    }
}

/// Bakes a UsdGeomMesh's points/topology into a Bevy [`Mesh`] and attaches
/// [`Mesh3d`] + a default [`StandardMaterial`]. No-op (with a warning) when the
/// render `Assets` are absent (headless) — the prim still projects, it just
/// carries no renderable geometry.
pub struct MeshRoute;

/// Opt-in for a host that services deferred canonical geometry through its residency worker.
#[derive(Resource, Default)]
pub struct DeferHiddenSourceMeshes;

/// Logical prim retained while its render mesh is absent.
#[derive(Component, Debug)]
pub struct DeferredSourceMesh;

pub fn source_mesh_is_hidden(world: &World, mut entity: Entity) -> bool {
    loop {
        match world.get::<Visibility>(entity) {
            Some(Visibility::Hidden) => return true,
            Some(Visibility::Visible) => return false,
            _ => match world.get::<ChildOf>(entity) {
                Some(parent) => entity = parent.parent(),
                None => return false,
            },
        }
    }
}

pub(crate) fn deferred_mesh_supported(ctx: &RouteCtx) -> bool {
    if ctx.type_name.as_deref() != Some("Mesh") {
        return false;
    }
    let Ok(prim) = ctx.stage.prim(ctx.path.clone()) else {
        return false;
    };
    if !matches!(prim.is_instance_proxy(), Ok(false)) || !matches!(prim.is_instance(), Ok(false)) {
        return false;
    }
    let mut path = Some(ctx.path.clone());
    while let Some(current) = path {
        if current.as_str() == "/" {
            break;
        }
        let Ok(ancestor) = ctx.stage.prim(current.clone()) else {
            return false;
        };
        if !matches!(ancestor.type_name(), Ok(name) if name.as_deref() != Some("SkelRoot"))
            || !matches!(ancestor.authored_api_schemas(), Ok(schemas) if schemas.iter().all(|schema| schema.as_str() == "MaterialBindingAPI"))
            || !matches!(ancestor.attributes(), Ok(attributes) if attributes.iter().all(|attribute| {
                attribute.time_sample_times().is_ok_and(|samples| samples.is_empty())
            }))
        {
            return false;
        }
        path = current.parent();
    }
    true
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MeshPatchAction {
    Ignore,
    UpdateExtent,
    Rebuild,
}

fn mesh_patch_action(changed: &[&str]) -> MeshPatchAction {
    if changed.is_empty() {
        return MeshPatchAction::Rebuild;
    }

    let mut extent_changed = false;
    for property in changed {
        if *property == "extent" {
            extent_changed = true;
        } else if is_geometry_property(property) || !is_known_non_geometry_property(property) {
            return MeshPatchAction::Rebuild;
        }
    }
    if extent_changed {
        MeshPatchAction::UpdateExtent
    } else {
        MeshPatchAction::Ignore
    }
}

fn is_geometry_property(property: &str) -> bool {
    matches!(
        property,
        "points"
            | "faceVertexCounts"
            | "faceVertexIndices"
            | "normals"
            | "normals:indices"
            | "normals:interpolation"
            | "orientation"
            | "subdivisionScheme"
            | "primvars:st"
            | "primvars:st:indices"
            | "primvars:st:interpolation"
            | "primvars:st0"
            | "primvars:st0:indices"
            | "primvars:st0:interpolation"
            | "primvars:displayColor"
            | "primvars:displayColor:indices"
            | "primvars:displayColor:interpolation"
            | "primvars:displayOpacity"
            | "primvars:displayOpacity:indices"
            | "primvars:displayOpacity:interpolation"
    )
}

fn is_known_non_geometry_property(property: &str) -> bool {
    property.starts_with("xformOp:")
        || matches!(
            property,
            "resetXformStack"
                | "xformOpOrder"
                | "visibility"
                | "purpose"
                | "kind"
                | "ui:displayName"
                | "doubleSided"
                | "documentation"
                | "comment"
                | "displayName"
                | "assetInfo"
                | "customData"
        )
        || property.starts_with("material:binding")
        || property.starts_with("customData:")
        || property.starts_with("bevy:")
}

fn update_extent(ctx: &RouteCtx, world: &mut World, entity: Entity) {
    let extent = read_mesh_extent(ctx.stage, ctx.path).ok().flatten();
    let Ok(mut entity) = world.get_entity_mut(entity) else {
        return;
    };
    if let Some([min, max]) = extent {
        entity.insert(UsdLocalExtent { min, max });
    } else {
        entity.remove::<UsdLocalExtent>();
    }
}

impl MeshRoute {
    /// Bake + attach; returns whether a `Mesh3d` was inserted.
    pub(crate) fn attach(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) -> bool {
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
        if let Some(seed) = world
            .get_resource_mut::<ProjectionSeed>()
            .and_then(|mut seeds| seeds.take_mesh(ctx.prim_str()))
            && world.resource::<Assets<Mesh>>().contains(&seed.handle)
        {
            let material = super::fallback_material(world);
            let attached = if let Ok(mut e) = world.get_entity_mut(entity) {
                if let Some((min, max)) = seed.local_extent {
                    e.insert(UsdLocalExtent { min, max });
                } else {
                    e.remove::<UsdLocalExtent>();
                }
                e.insert((Mesh3d(seed.handle.clone()), MeshMaterial3d(material)));
                true
            } else {
                false
            };
            if attached {
                track_mesh_projection(world, entity, &seed.handle);
            }
            return attached;
        }
        let profile_enabled = world
            .get_resource::<GeometryProfile>()
            .is_some_and(|profile| profile.enabled);
        if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
            counters.animation_read_mesh_calls(1);
        }
        let read_start = profile_enabled.then(Instant::now);
        let read_result = read_mesh(ctx.stage, ctx.path);
        let read_mesh_ms = read_start
            .map(|started| started.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or_default();
        let Ok(Some(read)) = read_result else {
            return false;
        };
        bevy::log::trace!(
            target: "usd_bevy::route::geom",
            "{}: mesh {} points -> Mesh3d",
            ctx.prim_str(),
            read.points.len()
        );
        let source_key = world
            .get_resource::<ProjectionCache>()
            .and_then(|_| source_mesh_key(&read));
        let source_hit = source_key
            .as_ref()
            .and_then(|key| lookup_source_mesh(world, key));
        let source_cache_lookup = source_key.is_some();
        let source_cache_hit = source_hit.is_some();
        let (mesh_handle, build_metrics, intern_metrics) = if let Some(mesh_handle) = source_hit {
            (mesh_handle, Default::default(), Default::default())
        } else {
            if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
                counters.animation_mesh_allocations(1);
            }
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
            if let Some(key) = source_key {
                remember_source_mesh(world, key, mesh_handle.clone());
            }
            (mesh_handle, build_metrics, intern_metrics)
        };
        if profile_enabled {
            let mut profile = world.resource_mut::<GeometryProfile>();
            record_mesh_sample(
                &mut profile,
                hash_prim_path(ctx.path.as_str()),
                read_mesh_ms,
                build_metrics,
                intern_metrics,
                !source_cache_hit,
                source_cache_lookup,
                source_cache_hit,
            );
        }
        let material = super::fallback_material(world);
        let mesh_handle = mesh_handle;
        let attached = if let Ok(mut e) = world.get_entity_mut(entity) {
            if let Some([min, max]) = read.extent {
                e.insert(UsdLocalExtent { min, max });
            } else {
                e.remove::<UsdLocalExtent>();
            }
            e.insert((Mesh3d(mesh_handle.clone()), MeshMaterial3d(material)));
            true
        } else {
            false
        };
        if attached {
            track_mesh_projection(world, entity, &mesh_handle);
        }
        attached
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
        if world.contains_resource::<DeferHiddenSourceMeshes>()
            && world.contains_resource::<super::material::InitialMaterialMemo>()
            && !world
                .get_resource::<ProjectionSeed>()
                .is_some_and(|seed| seed.has_mesh(ctx.prim_str()))
            && world
                .get_resource::<super::SchemaRegistry>()
                .is_some_and(super::SchemaRegistry::has_only_builtin_routes)
            && source_mesh_is_hidden(world, entity)
            && deferred_mesh_supported(ctx)
            && let Ok(Some([min, max])) = read_mesh_extent(ctx.stage, ctx.path)
        {
            world
                .entity_mut(entity)
                .insert((UsdLocalExtent { min, max }, DeferredSourceMesh));
            return;
        }
        self.attach(ctx, world, entity);
    }

    fn patch(&self, ctx: &RouteCtx, world: &mut World, entity: Entity, changed: &[&str]) {
        if world.get::<DeferredSourceMesh>(entity).is_some() {
            update_extent(ctx, world, entity);
            if world.get::<UsdLocalExtent>(entity).is_none() || !deferred_mesh_supported(ctx) {
                world.entity_mut(entity).remove::<DeferredSourceMesh>();
                self.attach(ctx, world, entity);
                super::material::MaterialRoute.project(ctx, world, entity);
            }
            return;
        }
        match mesh_patch_action(changed) {
            MeshPatchAction::Ignore => {}
            MeshPatchAction::UpdateExtent => update_extent(ctx, world, entity),
            MeshPatchAction::Rebuild => {
                self.attach(ctx, world, entity);
            }
        }
    }
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

#[cfg(test)]
#[path = "../../tests/unit/route/geom_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/route/hierarchy_tests.rs"]
mod hierarchy_tests;
