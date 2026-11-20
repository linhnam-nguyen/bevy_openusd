//! Payload route (PLAN Phase 3) — the `queue_spawn_scene` analog.
//!
//! A prim whose payload is currently *unloaded* projects as a placeholder
//! carrying [`UsdPayloadUnloaded`] (its payloaded subtree is absent). An app
//! streams it in with [`LiveStage::load_payload`](crate::live::LiveStage::load_payload)
//! and back out with `unload_payload`; both are composition changes, so the
//! live loop reconciles — materializing or despawning the subtree — and this
//! route adds/removes the marker to match.

use bevy::prelude::*;

use super::{PrimRoute, RouteCtx};

/// Marker on a prim whose payload is not currently loaded.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct UsdPayloadUnloaded;

/// Diagnostic attached to a prim entity when inspecting its payload state fails.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct UsdPayloadError(pub String);

/// Tracks payload load state as a marker component.
pub struct PayloadRoute;

fn is_unloaded(ctx: &RouteCtx) -> anyhow::Result<bool> {
    if !ctx.path.is_abs()
        || (!ctx.path.is_abs_root() && (!ctx.path.is_prim() || ctx.path.has_variant_selection()))
    {
        anyhow::bail!("invalid prim path for payload state query: {}", ctx.path);
    }
    // `is_loaded()` short-circuits to `true` when the stage has no load rules
    // (the fully-loaded common case), so this is cheap on ordinary stages.
    let prim = ctx.stage.prim(ctx.path.clone())?;
    let loaded = prim.is_loaded()?;
    Ok(!loaded)
}

impl PrimRoute for PayloadRoute {
    fn matches(&self, _ctx: &RouteCtx) -> bool {
        // Runs on every prim so it can also *remove* the marker when a payload
        // is loaded (matching on unloaded-only would strand the marker).
        true
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let state = is_unloaded(ctx);
        if let Ok(mut e) = world.get_entity_mut(entity) {
            match state {
                Ok(true) => {
                    e.remove::<UsdPayloadError>();
                    e.insert(UsdPayloadUnloaded);
                }
                Ok(false) => {
                    e.remove::<UsdPayloadError>();
                    e.remove::<UsdPayloadUnloaded>();
                }
                Err(err) => {
                    bevy::log::warn!(
                        "PayloadRoute failed to inspect payload state for {}: {err:#}",
                        ctx.path
                    );
                    e.insert(UsdPayloadError(format!("{err:#}")));
                }
            }
        }
    }

    fn patch(&self, ctx: &RouteCtx, world: &mut World, entity: Entity, _changed: &[&str]) {
        self.project(ctx, world, entity);
    }
}

#[cfg(test)]
#[path = "../../tests/unit/route/payload_tests.rs"]
mod tests;
