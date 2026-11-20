//! World-space affine propagation for non-TRS residual transforms (`U40`) and
//! `!resetXformStack!` (`U41`).
//!
//! Bevy's built-in transform propagation composes [`Transform`] (TRS) up the
//! [`ChildOf`] hierarchy. When a USD prim authors a non-TRS local matrix (for
//! example, a shear or reflection that cannot be represented losslessly as
//! `Transform`) or `!resetXformStack!` (which resets inherited parent
//! transforms while keeping the stage root up-axis basis applied once),
//! [`propagate_usd_affine_transforms_system`] recomputes [`GlobalTransform`]
//! along affected hierarchy chains after Bevy's standard transform propagation.

use bevy::ecs::hierarchy::{ChildOf, Children};
use bevy::math::Affine3A;
use bevy::prelude::*;

use super::{UsdFullLocalAffine, UsdResetXformStack, UsdResidualAffine};
use crate::prim_ref::UsdPrimRef;

/// Recomputes [`GlobalTransform`] for subtrees containing [`UsdResidualAffine`],
/// [`UsdFullLocalAffine`] (`U40`), or [`UsdResetXformStack`] (`U41`).
pub fn propagate_usd_affine_transforms_system(
    special_check: Query<
        (),
        Or<(
            With<UsdResidualAffine>,
            With<UsdFullLocalAffine>,
            With<UsdResetXformStack>,
        )>,
    >,
    roots: Query<
        (Entity, &Transform, Option<&UsdPrimRef>, Option<&Children>),
        Without<ChildOf>,
    >,
    nodes: Query<(
        &Transform,
        Option<&UsdResidualAffine>,
        Option<&UsdFullLocalAffine>,
        Option<&UsdResetXformStack>,
        Option<&Children>,
    )>,
    mut globals: Query<&mut GlobalTransform>,
) {
    if special_check.is_empty() {
        return;
    }

    for (root_entity, root_transform, prim_ref, children) in &roots {
        let root_affine = root_transform.compute_affine();
        let stage_root_affine = if prim_ref.is_some_and(|p| p.path == "/") {
            root_affine
        } else {
            Affine3A::IDENTITY
        };
        if let Some(children) = children {
            for child in children.iter() {
                propagate_node(
                    child,
                    root_affine,
                    stage_root_affine,
                    false,
                    &nodes,
                    &mut globals,
                );
            }
        }
        let _ = root_entity;
    }
}

fn propagate_node(
    entity: Entity,
    parent_world: Affine3A,
    stage_root_affine: Affine3A,
    ancestor_dirty: bool,
    nodes: &Query<(
        &Transform,
        Option<&UsdResidualAffine>,
        Option<&UsdFullLocalAffine>,
        Option<&UsdResetXformStack>,
        Option<&Children>,
    )>,
    globals: &mut Query<&mut GlobalTransform>,
) {
    let Ok((transform, residual, full_local, reset_stack, children)) = nodes.get(entity) else {
        return;
    };

    let has_special = residual.is_some() || full_local.is_some() || reset_stack.is_some();
    let dirty = ancestor_dirty || has_special;
    let base_parent = if reset_stack.is_some() {
        stage_root_affine
    } else {
        parent_world
    };
    let local_affine = if let Some(full) = full_local {
        full.0
    } else {
        let mut trs_affine = transform.compute_affine();
        if let Some(residual) = residual {
            trs_affine = trs_affine * residual.0;
        }
        trs_affine
    };
    let world_affine = base_parent * local_affine;

    if dirty && let Ok(mut global) = globals.get_mut(entity) {
        *global = GlobalTransform::from(world_affine);
    }

    if let Some(children) = children {
        for child in children.iter() {
            propagate_node(
                child,
                world_affine,
                stage_root_affine,
                dirty,
                nodes,
                globals,
            );
        }
    }
}
