use anyhow::Result;
use bevy::prelude::*;
use openusd::usd::Stage;
use std::collections::HashSet;

use super::animation::{AnimatedPrims, prim_is_animated};
use super::index::PrimEntities;
use super::path::{is_descendant_or_self, parent_path, validate_prim_path};
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

/// The traversal predicate for projection: active + defined + non-abstract, but
/// **not** requiring `LOADED` (unlike `PrimPredicate::default()`). This projects
/// a prim whose payload is *unloaded* as a placeholder (its payloaded children
/// stay absent until [`LiveStage::load_payload`]). For fully-loaded stages this
/// is identical to the default predicate.
pub(super) fn traverse_predicate() -> openusd::usd::PrimPredicate {
    use openusd::usd::PrimStatus;
    openusd::usd::PrimPredicate::new(
        PrimStatus::ACTIVE.union(PrimStatus::DEFINED),
        PrimStatus::ABSTRACT,
    )
}

/// Collects all valid, projected prim paths within a subtree rooted at `root`.
///
/// Uses the canonical projection predicate (`ACTIVE | DEFINED & ~ABSTRACT`) so
/// that subtree reconciliation and semantic extraction see exactly the prims
/// that the renderer projects.
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
pub fn project_stage(world: &mut World, live: &LiveStage, map: &mut PrimEntities) {
    let stage = &live.stage;
    let registry = registry_of(world);
    let root = world
        .spawn((
            UsdPrimRef {
                path: "/".to_string(),
            },
            Transform::from_rotation(stage_up_axis(stage)),
            Visibility::default(),
        ))
        .id();
    map.insert("/", root);

    let mut prim_count = 0usize;
    let mut animated: HashSet<String> = HashSet::new();
    let _ = stage.traverse(traverse_predicate(), |path: &openusd::sdf::Path| {
        let parent = map.entity(parent_path(path.as_str())).unwrap_or(root);
        let entity = world
            .spawn((
                UsdPrimRef {
                    path: path.as_str().to_string(),
                },
                ChildOf(parent),
            ))
            .id();
        map.insert(path.as_str().to_string(), entity);
        prim_count += 1;
        if prim_is_animated(stage, path) {
            animated.insert(path.as_str().to_string());
        }
        registry.project_prim(stage, path, world, entity);
    });
    bevy::log::info!(
        session = live.session_id(),
        prims = prim_count,
        animated = animated.len(),
        "projected USD stage"
    );
    world.insert_resource(AnimatedPrims(animated));
    let _ = live.drain_change_batch();
}

/// One-shot projection the first frame a `LiveStage` is present.
pub(super) fn project_on_load_system(world: &mut World) {
    if world.get_non_send::<LiveStage>().is_none() {
        return;
    }
    if let Some(map) = world.get_resource::<PrimEntities>() {
        if !map.is_empty() {
            return;
        }
    }
    let Some(live) = world.remove_non_send::<LiveStage>() else {
        return;
    };
    let mut map = world.remove_resource::<PrimEntities>().unwrap_or_default();
    project_stage(world, &live, &mut map);
    world.insert_resource(map);
    world.insert_non_send(live);
}
