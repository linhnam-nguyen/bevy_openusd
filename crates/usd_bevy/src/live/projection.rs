use anyhow::{Result, ensure};
use bevy::mesh::Mesh;
use bevy::prelude::*;
use openusd::usd::Stage;
use std::collections::{BTreeSet, HashSet};

use super::animation::{AnimatedPrims, prim_is_animated};
use super::index::PrimEntities;
use super::native_animation;
use super::native_instance_dependency::NativeInstanceDependencyIndex;
use super::path::{PathStore, is_descendant_or_self, parent_path, validate_prim_path};
use super::performance::PerformanceCounters;
use super::projection_plan::ProjectionPlan;
use super::stage::LiveStage;
use crate::prim_ref::UsdPrimRef;
use crate::route::SchemaRegistry;

pub(super) fn to_bevy_transform(t: crate::read::xform::Transform3) -> Transform {
    Transform {
        translation: Vec3::from_array(t.translate),
        rotation: Quat::from_array(t.rotate),
        scale: Vec3::from_array(t.scale),
    }
}

/// Rotation mapping the stage's authored up-axis onto Bevy's Y-up world. USD
/// defaults to Y-up; Z-up content (common for robotics / CAD assets) is rotated
/// -90° about X so +Z becomes +Y. Applied once on the stage-root entity so the
/// whole composed scene stands upright on the ground grid.
pub(super) fn stage_up_axis(stage: &Stage) -> Quat {
    let is_z = matches!(
        stage.stage_metadata("upAxis").ok().flatten(),
        Some(openusd::sdf::Value::Token(t)) if t.as_str() == "Z"
    );
    if is_z {
        Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2)
    } else {
        Quat::IDENTITY
    }
}

/// The traversal predicate for projection: active + defined + non-abstract,
/// descending through native instance proxies, but **not** requiring `LOADED`
/// (unlike `PrimPredicate::default()`). This projects a prim whose payload is
/// *unloaded* as a placeholder (its payloaded children stay absent until
/// [`LiveStage::load_payload`]). For fully-loaded stages this is the default
/// predicate with instance proxies enabled.
pub(super) fn traverse_predicate() -> openusd::usd::PrimPredicate {
    use openusd::usd::PrimStatus;
    openusd::usd::PrimPredicate::new(
        PrimStatus::ACTIVE.union(PrimStatus::DEFINED),
        PrimStatus::ABSTRACT,
    )
    .with_instance_proxies(true)
}

/// Collects all valid, projected prim paths within a subtree rooted at `root`.
///
/// Uses the canonical proxy-aware projection predicate (`ACTIVE | DEFINED &
/// ~ABSTRACT`) so subtree reconciliation and semantic extraction see exactly
/// the scene-scoped prims that the renderer projects.
pub fn collect_stage_subtree_paths(stage: &Stage, root: &str) -> Result<Vec<String>> {
    let normalized_root = validate_prim_path(root)?;
    let mut collected = Vec::new();
    stage.traverse(traverse_predicate(), |path: &openusd::sdf::Path| {
        let path_str = path.as_str();
        if is_descendant_or_self(&normalized_root, path_str) {
            collected.push(path_str.to_string());
        }
    })?;
    Ok(collected)
}

/// A render payload extracted for one explicitly requested prim path.
#[derive(Debug)]
pub struct TargetedRenderPayload {
    pub path: String,
    pub mesh: Mesh,
    pub local_bounds: Option<usd_model::Bounds3>,
}

/// Read renderable geometry only for the requested paths.
///
/// The caller owns persistence. This boundary deliberately does not walk the
/// stage or inspect cache objects, so a selection can extract one uncached
/// object without re-projecting or rehashing already-resident geometry.
pub fn extract_render_payloads_for_paths(
    stage: &Stage,
    requested_paths: &[&str],
) -> Result<Vec<TargetedRenderPayload>> {
    let mut paths = BTreeSet::new();
    for raw_path in requested_paths {
        let path = validate_prim_path(raw_path)?;
        ensure!(path != "/", "targeted extraction cannot request stage root");
        let sdf_path = openusd::sdf::path(&path)?;
        let prim = stage.prim(sdf_path);
        ensure!(prim.is_active()?, "targeted prim {path} is inactive");
        ensure!(prim.is_defined()?, "targeted prim {path} is undefined");
        ensure!(!prim.is_abstract()?, "targeted prim {path} is abstract");
        paths.insert(path);
    }

    let mut payloads = Vec::new();
    for path in paths {
        let sdf_path = openusd::sdf::path(&path)?;
        let Some(read) = crate::read::geom::read_mesh(stage, &sdf_path)? else {
            continue;
        };
        let local_bounds = read
            .extent
            .map(|[min, max]| usd_model::Bounds3 {
                min: min.map(f64::from),
                max: max.map(f64::from),
            })
            .or_else(|| bounds_from_points(&read.points));
        payloads.push(TargetedRenderPayload {
            path,
            mesh: crate::mesh::mesh_from_usd(&read),
            local_bounds,
        });
    }
    Ok(payloads)
}

fn bounds_from_points(points: &[[f32; 3]]) -> Option<usd_model::Bounds3> {
    let first = points.first().copied()?;
    let mut min = first.map(f64::from);
    let mut max = min;
    for point in points.iter().skip(1) {
        let point = point.map(f64::from);
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    Some(usd_model::Bounds3 { min, max })
}

/// Project exact requested paths and their namespace ancestors.
///
/// Existing ancestors are reused. Existing requested entities are re-routed
/// after payload loading so placeholders receive their newly composed schema
/// data, while no full-stage walk or change-batch drain is performed.
pub fn project_paths(
    world: &mut World,
    live: &LiveStage,
    map: &mut PrimEntities,
    requested_paths: &[&str],
) -> Result<usize> {
    let normalized = requested_paths
        .iter()
        .map(|path| validate_prim_path(path))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        normalized.iter().all(|path| path != "/"),
        "targeted projection cannot request stage root"
    );
    if normalized.is_empty() {
        return Ok(0);
    }
    let requested = normalized
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    for path in &normalized {
        if path == "/" {
            continue;
        }
        let sdf_path = openusd::sdf::path(path)?;
        if !live.stage.prim(sdf_path).is_loaded()? {
            live.load_payload(path);
        }
    }
    let plan =
        ProjectionPlan::from_paths(&live.stage, &requested.iter().copied().collect::<Vec<_>>())?;
    let registry = registry_of(world);
    world.init_resource::<PathStore>();
    let mut materialized = 0;
    for entry in plan.entries() {
        let existing = {
            let paths = world.resource::<PathStore>();
            map.entity(&paths, entry.path())
        };
        if let Some(entity) = existing {
            if requested.contains(entry.path()) {
                if let Ok(path) = openusd::sdf::path(entry.path()) {
                    registry.project_prim(&live.stage, &path, world, entity);
                }
            }
            continue;
        }
        let parent = {
            let paths = world.resource::<PathStore>();
            entry
                .parent_index()
                .and_then(|_| map.entity(&paths, parent_path(entry.path())))
                .or_else(|| map.entity(&paths, "/"))
        };
        project_plan_entry(world, &live.stage, &registry, map, entry, parent);
        materialized += 1;
    }
    if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
        counters.projection_paths_planned(plan.len() as u64);
    }
    Ok(materialized)
}

/// Snapshot the registry out of the world (Arc-cheap `Clone`), falling back to
/// the built-in routes when none is installed — so direct `project_stage` /
/// `apply_changes` calls in tests work without wiring a registry.
pub(super) fn registry_of(world: &World) -> SchemaRegistry {
    world
        .get_resource::<SchemaRegistry>()
        .cloned()
        .unwrap_or_else(SchemaRegistry::builtin)
}

/// Project every prim in the stage into an entity (`UsdPrimRef` +
/// `Transform`), recording the path↔entity bimap. Idempotent only on an
/// empty world — call once on load.
/// Initial stage projection timing and prim count metrics.
#[derive(Resource, Debug, Clone, Default)]
pub struct ProjectionStats {
    pub initial_projection_ms: Option<f64>,
    pub initial_projection_prims: u64,
    pub stage_traversal_ms: Option<f64>,
    pub mesh_generation_ms: Option<f64>,
    pub primvar_expansion_ms: Option<f64>,
    pub normal_generation_ms: Option<f64>,
    pub material_resolve_ms: Option<f64>,
}

pub fn project_stage(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let start = std::time::Instant::now();
    let stage = &live.stage;
    let registry = registry_of(world);
    world.init_resource::<PathStore>();
    let Ok(plan) = ProjectionPlan::from_stage(stage) else {
        bevy::log::error!("[projection] failed to build deterministic projection plan");
        return;
    };
    if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
        counters.projection_full_stage_walks(1);
        counters.projection_paths_planned(plan.len() as u64);
    }
    let mut animated: HashSet<String> = HashSet::new();
    let traversal_start = std::time::Instant::now();
    for entry in plan.entries() {
        let parent = {
            let paths = world.resource::<PathStore>();
            entry
                .parent_index()
                .and_then(|_| map.entity(&paths, parent_path(entry.path())))
                .or_else(|| map.entity(&paths, "/"))
        };
        let entity = project_plan_entry(world, stage, &registry, map, entry, parent);
        if entry.path() != "/"
            && openusd::sdf::path(entry.path())
                .ok()
                .is_some_and(|path| prim_is_animated(stage, &path))
        {
            animated.insert(entry.path().to_string());
        }
        let _ = entity;
    }
    let traversal_duration = traversal_start.elapsed().as_secs_f64() * 1000.0;
    let duration = start.elapsed().as_secs_f64() * 1000.0;

    bevy::log::info!(
        session = live.session_id(),
        prims = plan.len().saturating_sub(1),
        animated = animated.len(),
        duration_ms = duration,
        "projected USD stage"
    );
    world.insert_resource(AnimatedPrims(animated));
    native_animation::rebuild(world, live, map);
    world.insert_resource(ProjectionStats {
        initial_projection_ms: Some(duration),
        initial_projection_prims: plan.len().saturating_sub(1) as u64,
        stage_traversal_ms: Some(traversal_duration),
        mesh_generation_ms: None,
        primvar_expansion_ms: None,
        normal_generation_ms: None,
        material_resolve_ms: None,
    });
    world.init_resource::<NativeInstanceDependencyIndex>();
    let rebuild = world.resource_scope(
        |world, mut dependencies: Mut<NativeInstanceDependencyIndex>| {
            let mut paths = world.resource_mut::<PathStore>();
            dependencies.rebuild(&mut paths, stage)
        },
    );
    if let Err(error) = rebuild {
        bevy::log::warn!("[projection] native instance dependency index rebuild failed: {error:#}");
    }
    let _ = live.drain_change_batch();
}

pub(super) fn project_plan_entry(
    world: &mut World,
    stage: &Stage,
    registry: &SchemaRegistry,
    map: &mut PrimEntities,
    entry: &super::projection_plan::ProjectionPlanEntry,
    parent: Option<Entity>,
) -> Entity {
    let entity = if entry.path() == "/" {
        world
            .spawn((
                UsdPrimRef {
                    path: "/".to_string(),
                },
                Transform::from_rotation(stage_up_axis(stage)),
                Visibility::default(),
            ))
            .id()
    } else {
        let parent = parent.expect("parent-before-child projection plan has a parent");
        world
            .spawn((
                UsdPrimRef {
                    path: entry.path().to_string(),
                },
                ChildOf(parent),
            ))
            .id()
    };
    world.resource_scope(|_world, mut paths: Mut<PathStore>| {
        map.insert(&mut paths, entry.path(), entity);
    });
    if let Some(mut counters) = world.get_resource_mut::<PerformanceCounters>() {
        counters.projection_paths_materialized(1);
    }
    if entry.path() != "/"
        && let Ok(path) = openusd::sdf::path(entry.path())
    {
        registry.project_prim(stage, &path, world, entity);
    }
    entity
}
