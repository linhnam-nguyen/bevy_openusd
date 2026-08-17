//! Render / procedural / UI coverage routes (PLAN Phase 7): `RenderSettings`,
//! `GenerativeProcedural` and `Backdrop` prims → typed marker components.
//!
//! These schemas have no direct Bevy runtime equivalent (render config lives in
//! Bevy's own render graph; procedurals need an evaluator; backdrops are DCC
//! editor annotations). Like [`super::physics`] and [`super::audio`], the
//! routes project **data markers** so an app can discover them and act — read
//! render config, invoke a procedural evaluator, show a graph-editor note —
//! without re-walking the stage.

use bevy::prelude::*;

use openusd::schemas::proc::GenerativeProcedural;
use openusd::schemas::render::RenderSettings;
use openusd::schemas::ui::Backdrop;
use openusd::sdf::Value;

use super::{PrimRoute, RouteCtx};

/// A `RenderSettings` prim: top-level render configuration.
#[derive(Component, Debug, Clone, Default)]
pub struct UsdRenderSettings {
    /// `includedPurposes` (e.g. `["default", "render"]`).
    pub included_purposes: Vec<String>,
    /// `renderingColorSpace` token, if authored.
    pub color_space: Option<String>,
}

/// A `GenerativeProcedural` prim: geometry produced by an external evaluator.
#[derive(Component, Debug, Clone, Default)]
pub struct UsdProcedural {
    /// `proceduralSystem`: which evaluator owns this prim.
    pub system: Option<String>,
}

/// A `Backdrop` prim: a DCC graph-editor annotation region.
#[derive(Component, Debug, Clone, Default)]
pub struct UsdBackdrop {
    /// `ui:description` free-text note.
    pub description: Option<String>,
}

fn token_string(v: Option<Value>) -> Option<String> {
    match v? {
        Value::Token(t) => Some(t.as_str().to_string()),
        Value::String(s) => Some(s),
        _ => None,
    }
}

fn token_vec(v: Option<Value>) -> Vec<String> {
    match v {
        Some(Value::TokenVec(a)) => a.iter().map(|t| t.as_str().to_string()).collect(),
        Some(Value::StringVec(a)) => a,
        _ => Vec::new(),
    }
}

/// Projects `RenderSettings` prims as [`UsdRenderSettings`] markers.
pub struct RenderSettingsRoute;

impl PrimRoute for RenderSettingsRoute {
    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.as_deref() == Some("RenderSettings")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let Ok(Some(rs)) = RenderSettings::get(ctx.stage, ctx.path.clone()) else {
            return;
        };
        let marker = UsdRenderSettings {
            included_purposes: token_vec(rs.included_purposes_attr().get::<Value>().ok().flatten()),
            color_space: token_string(
                rs.rendering_color_space_attr()
                    .get::<Value>()
                    .ok()
                    .flatten(),
            ),
        };
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(marker);
        }
    }
}

/// Projects `GenerativeProcedural` prims as [`UsdProcedural`] markers.
pub struct ProceduralRoute;

impl PrimRoute for ProceduralRoute {
    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.as_deref() == Some("GenerativeProcedural")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let Ok(Some(p)) = GenerativeProcedural::get(ctx.stage, ctx.path.clone()) else {
            return;
        };
        let marker = UsdProcedural {
            system: token_string(p.procedural_system_attr().get::<Value>().ok().flatten()),
        };
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(marker);
        }
    }
}

/// Projects `Backdrop` prims as [`UsdBackdrop`] markers.
pub struct BackdropRoute;

impl PrimRoute for BackdropRoute {
    fn matches(&self, ctx: &RouteCtx) -> bool {
        ctx.type_name.as_deref() == Some("Backdrop")
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let Ok(Some(b)) = Backdrop::get(ctx.stage, ctx.path.clone()) else {
            return;
        };
        let marker = UsdBackdrop {
            description: token_string(b.description_attr().get::<Value>().ok().flatten()),
        };
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(marker);
        }
    }
}
