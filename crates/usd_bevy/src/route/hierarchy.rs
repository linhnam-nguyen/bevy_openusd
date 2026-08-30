//! Semantic hierarchy metadata projected from the authoritative USD Stage.

use bevy::prelude::{Entity, World};
use openusd::sdf::Value;

use super::super::RouteCtx;
use super::UsdDisplayName;
use crate::prim_ref::{
    USDHUB_HIERARCHY_ROLE_METADATA, USDHUB_TRANSPARENT_SOURCE_ROLE, UsdTransparentHierarchyNode,
};

/// Project semantic labels and explicit USDHub implementation roles alongside
/// the normal visibility route. These components are disposable projections;
/// the composed Stage remains authoritative for both values.
pub(super) fn apply_metadata(ctx: &RouteCtx, world: &mut World, entity: Entity) {
    let display_name = read_display_name(ctx);
    let transparent_source = ctx
        .stage
        .prim(ctx.path.clone())
        .custom_data()
        .ok()
        .flatten()
        .and_then(|value| match value {
            Value::Dictionary(data) => data
                .get(USDHUB_HIERARCHY_ROLE_METADATA)
                .and_then(Value::as_str)
                .map(|value| value == USDHUB_TRANSPARENT_SOURCE_ROLE),
            _ => None,
        })
        .unwrap_or(false);

    let Ok(mut entity) = world.get_entity_mut(entity) else {
        return;
    };
    if let Some(display_name) = display_name {
        entity.insert(UsdDisplayName(display_name));
    } else {
        entity.remove::<UsdDisplayName>();
    }
    if transparent_source {
        entity.insert(UsdTransparentHierarchyNode);
    } else {
        entity.remove::<UsdTransparentHierarchyNode>();
    }
}

/// Read both representations found in USDHub's existing files: ordinary USD
/// `ui:displayName` attributes and the prim metadata field authored by the
/// storage-v2 project layer. The layer scan preserves composed-stage reading
/// for the latter without exposing OpenUSD's internal Stage field resolver.
fn read_display_name(ctx: &RouteCtx) -> Option<String> {
    let as_text = |value: Value| match value {
        Value::String(value) => Some(value),
        Value::Token(value) => Some(value.as_str().to_owned()),
        _ => None,
    };
    ctx.stage
        .prim(ctx.path.clone())
        .attribute("ui:displayName")
        .get::<Value>()
        .ok()
        .flatten()
        .and_then(as_text)
        .or_else(|| {
            ctx.stage
                .layer_stack()
                .into_iter()
                .filter_map(|identifier| {
                    let layer = ctx.stage.layer(&identifier)?;
                    let spec = layer.prim(ctx.path)?;
                    spec.field("ui:displayName").ok().flatten()
                })
                .find_map(as_text)
        })
}
