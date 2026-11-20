//! Transform route: any prim's composed local transform → Bevy [`Transform`],
//! plus optional [`UsdResidualAffine`] (`U40`) and [`UsdResetXformStack`] (`U41`).
//!
//! The composed *world* transform comes from Bevy's transform propagation up
//! the projected prim hierarchy (see [`crate::live::project_stage`]), augmented
//! by [`propagation::propagate_usd_affine_transforms_system`] when residual
//! affine factors or `!resetXformStack!` are authored.

#[path = "xform/propagation.rs"]
pub mod propagation;

use bevy::math::Affine3A;
use bevy::prelude::*;

use super::{PrimRoute, RouteCtx};
use crate::read::xform::{LocalXformEvaluation, Transform3, read_local_xform_at, read_transform_at};

/// Non-TRS residual affine factor (`T_trs^-1 * M_local`) when a prim's composed
/// local 4×4 matrix contains shear or non-orthogonal reflection (`U40`).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct UsdResidualAffine(pub Affine3A);

/// Full local affine override (`M_local`) when a prim's composed local 4×4
/// matrix contains non-TRS shear/reflection combined with a singular or
/// near-singular determinant where `T_trs^-1 * M_local` cannot reconstruct
/// `M_local` losslessly (`F-C03-I1-02`, `U40`).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct UsdFullLocalAffine(pub Affine3A);

/// Marker inserted when a prim's `xformOpOrder` includes `!resetXformStack!`
/// (`U41`), instructing world-transform propagation to reset inherited parent
/// transforms while preserving the stage root up-axis basis once.
#[derive(Component, Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct UsdResetXformStack;

/// Maps an `xformOp` stack to [`Transform`]. Applies to every prim — an
/// unauthored transform reads as identity, matching USD's Xformable fallback.
pub struct XformRoute;

fn to_bevy_transform(t: Transform3) -> Transform {
    Transform {
        translation: Vec3::from_array(t.translate),
        rotation: Quat::from_array(t.rotate),
        scale: Vec3::from_array(t.scale),
    }
}

/// The prim's local transform, or identity when none is authored / the read
/// fails.
pub fn transform_of(ctx: &RouteCtx) -> Transform {
    read_transform_at(ctx.stage, ctx.path, ctx.time)
        .ok()
        .flatten()
        .map(to_bevy_transform)
        .unwrap_or_default()
}

pub(crate) fn apply_local_xform_evaluation(
    world: &mut World,
    entity: Entity,
    eval: Option<LocalXformEvaluation>,
) {
    let t = eval.map(|e| to_bevy_transform(e.trs)).unwrap_or_default();
    let residual = eval.and_then(|e| e.residual_affine);
    let full_local = eval.and_then(|e| e.full_local_affine);
    let resets = eval.is_some_and(|e| e.resets_xform_stack);
    if let Some(mut tr) = world.get_mut::<Transform>(entity) {
        *tr = t;
    } else if let Ok(mut e) = world.get_entity_mut(entity) {
        e.insert(t);
    }
    if let Ok(mut e) = world.get_entity_mut(entity) {
        match residual {
            Some(affine) => {
                e.insert(UsdResidualAffine(affine));
            }
            None => {
                e.remove::<UsdResidualAffine>();
            }
        }
        match full_local {
            Some(affine) => {
                e.insert(UsdFullLocalAffine(affine));
            }
            None => {
                e.remove::<UsdFullLocalAffine>();
            }
        }
        if resets {
            e.insert(UsdResetXformStack);
        } else {
            e.remove::<UsdResetXformStack>();
        }
    }
}

impl PrimRoute for XformRoute {
    fn matches(&self, _ctx: &RouteCtx) -> bool {
        true
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        match read_local_xform_at(ctx.stage, ctx.path, ctx.time) {
            Ok(eval) => apply_local_xform_evaluation(world, entity, eval),
            Err(err) => {
                log::warn!("rejecting invalid USD local xform on {}: {err}", ctx.prim_str());
                if world.get::<Transform>(entity).is_none()
                    && let Ok(mut e) = world.get_entity_mut(entity)
                {
                    e.insert(Transform::IDENTITY);
                }
            }
        }
    }

    fn patch(&self, ctx: &RouteCtx, world: &mut World, entity: Entity, changed: &[&str]) {
        // Only react to xformOp changes (`xformOp:*`, `xformOpOrder`). A patch
        // that touches nothing transform-related is a no-op here.
        let touches_xform = changed.is_empty()
            || changed
                .iter()
                .any(|p| p.starts_with("xformOp") || *p == "xformOpOrder");
        if !touches_xform {
            return;
        }
        match read_local_xform_at(ctx.stage, ctx.path, ctx.time) {
            Ok(eval) => apply_local_xform_evaluation(world, entity, eval),
            Err(err) => {
                log::warn!("rejecting invalid USD local xform patch on {}: {err}", ctx.prim_str());
            }
        }
    }
}
