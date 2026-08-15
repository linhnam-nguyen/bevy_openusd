//! Light route (PLAN P4 + Phase C): UsdLux prims → Bevy lights.
//!
//! * `DistantLight` → [`DirectionalLight`]
//! * `SphereLight` / `DiskLight` → [`PointLight`], or [`SpotLight`] when a
//!   `shaping:cone:angle` is authored
//! * `RectLight` / `CylinderLight` → [`PointLight`] **approximation** (Bevy has
//!   no true area light) plus a [`UsdAreaLight`] marker carrying the authored
//!   dimensions, so an app with an area-light backend can upgrade it.
//!
//! Photometric units differ between USD (author-defined intensity, exposure
//! stops) and Bevy (lux / lumens), so intensity is an **approximate** mapping
//! through the named scale constants below — colour and light *kind* project
//! faithfully; absolute brightness is a best-effort default the app can tune.

use bevy::prelude::*;
use std::f32::consts::PI;

use openusd::schemas::lux::{
    CylinderLight, DiskLight, DistantLight, Light, RectLight, ShapingAPI, SphereLight,
};
use openusd::sdf::Value;

use super::{PrimRoute, RouteCtx};

/// A UsdLux area light that Bevy can't represent natively. Carries the authored
/// shape so an app with a real area-light backend can build the exact light;
/// the route itself only approximates it with a [`PointLight`].
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub enum UsdAreaLight {
    /// `RectLight` — a `width` × `height` rectangle.
    Rect { width: f32, height: f32 },
    /// `CylinderLight` — a rod of the given `length` and `radius`.
    Cylinder { length: f32, radius: f32 },
}

/// USD `DistantLight` intensity → Bevy illuminance (lux). USD's default distant
/// intensity is 1; daylight in Bevy is ~10⁴ lux.
const DISTANT_LUX_SCALE: f32 = 10_000.0;
/// USD point/sphere intensity → Bevy luminous intensity (lumens), rough.
const POINT_LUMEN_SCALE: f32 = 1_000.0;

/// Decoded UsdLux common inputs.
struct LuxInputs {
    color: Color,
    intensity: f32,
    radius: Option<f32>,
    cone_angle: Option<f32>,
    /// Present for `RectLight`/`CylinderLight` — the area-light shape marker.
    area: Option<UsdAreaLight>,
}

/// Read the `UsdLuxLightAPI` inputs common to every light through the schema's
/// typed getters (`inputs:intensity`, `inputs:color`, `inputs:exposure`).
fn common_inputs<L: Light>(light: &L) -> (Color, f32) {
    let color = match light.color_attr().get::<Value>() {
        Ok(Some(Value::Vec3f(c))) => Color::linear_rgb(c.x, c.y, c.z),
        Ok(Some(Value::Vec3d(c))) => Color::linear_rgb(c.x as f32, c.y as f32, c.z as f32),
        _ => Color::WHITE,
    };
    let base = light
        .intensity_attr()
        .get::<f32>()
        .ok()
        .flatten()
        .unwrap_or(1.0);
    let exposure = light
        .exposure_attr()
        .get::<f32>()
        .ok()
        .flatten()
        .unwrap_or(0.0);
    (color, base * 2f32.powf(exposure))
}

fn cone_angle(ctx: &RouteCtx) -> Option<f32> {
    let shaping = ShapingAPI::get(ctx.stage, ctx.path.clone())
        .ok()
        .flatten()?;
    shaping.cone_angle_attr().get::<f32>().ok().flatten()
}

/// Maps UsdLux prims to Bevy light components.
pub struct LightRoute;

impl LightRoute {
    fn kind(ctx: &RouteCtx) -> Option<&'static str> {
        match ctx.type_name.as_deref()? {
            "DistantLight" => Some("distant"),
            "SphereLight" | "DiskLight" | "RectLight" | "CylinderLight" => Some("point"),
            _ => None,
        }
    }
}

impl PrimRoute for LightRoute {
    fn matches(&self, ctx: &RouteCtx) -> bool {
        Self::kind(ctx).is_some()
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let Some(kind) = Self::kind(ctx) else { return };
        // Read through the typed UsdLux schema for the prim's kind.
        let lux = match kind {
            "distant" => {
                let Ok(Some(light)) = DistantLight::get(ctx.stage, ctx.path.clone()) else {
                    return;
                };
                let (color, intensity) = common_inputs(&light);
                LuxInputs {
                    color,
                    intensity,
                    radius: None,
                    cone_angle: None,
                    area: None,
                }
            }
            _ => {
                // Sphere/Disk carry a radius; Rect/Cylinder are area lights we
                // approximate with a point at their center + a shape marker.
                let (color, intensity, radius, area) =
                    if let Ok(Some(l)) = SphereLight::get(ctx.stage, ctx.path.clone()) {
                        let (c, i) = common_inputs(&l);
                        (c, i, l.radius_attr().get::<f32>().ok().flatten(), None)
                    } else if let Ok(Some(l)) = DiskLight::get(ctx.stage, ctx.path.clone()) {
                        let (c, i) = common_inputs(&l);
                        (c, i, l.radius_attr().get::<f32>().ok().flatten(), None)
                    } else if let Ok(Some(l)) = RectLight::get(ctx.stage, ctx.path.clone()) {
                        let (c, i) = common_inputs(&l);
                        let w = l.width_attr().get::<f32>().ok().flatten().unwrap_or(1.0);
                        let h = l.height_attr().get::<f32>().ok().flatten().unwrap_or(1.0);
                        // Point radius ≈ the rectangle's half-extent.
                        (
                            c,
                            i,
                            Some(w.max(h) * 0.5),
                            Some(UsdAreaLight::Rect {
                                width: w,
                                height: h,
                            }),
                        )
                    } else if let Ok(Some(l)) = CylinderLight::get(ctx.stage, ctx.path.clone()) {
                        let (c, i) = common_inputs(&l);
                        let length = l.length_attr().get::<f32>().ok().flatten().unwrap_or(1.0);
                        let r = l.radius_attr().get::<f32>().ok().flatten().unwrap_or(0.5);
                        (
                            c,
                            i,
                            Some(r),
                            Some(UsdAreaLight::Cylinder { length, radius: r }),
                        )
                    } else {
                        return;
                    };
                LuxInputs {
                    color,
                    intensity,
                    radius,
                    cone_angle: cone_angle(ctx),
                    area,
                }
            }
        };
        let Ok(mut e) = world.get_entity_mut(entity) else {
            return;
        };
        // Replace any light this route added before (avoid stacking on resync).
        e.remove::<DirectionalLight>();
        e.remove::<PointLight>();
        e.remove::<SpotLight>();
        e.remove::<UsdAreaLight>();
        if let Some(area) = lux.area {
            e.insert(area);
        }

        match kind {
            "distant" => {
                e.insert(DirectionalLight {
                    color: lux.color,
                    illuminance: lux.intensity * DISTANT_LUX_SCALE,
                    ..default()
                });
            }
            _ => {
                let intensity = lux.intensity * POINT_LUMEN_SCALE;
                let radius = lux.radius.unwrap_or(0.0);
                if let Some(cone_deg) = lux.cone_angle {
                    let outer = (cone_deg * PI / 180.0).clamp(0.0, PI / 2.0);
                    e.insert(SpotLight {
                        color: lux.color,
                        intensity,
                        radius,
                        outer_angle: outer,
                        inner_angle: outer * 0.9,
                        ..default()
                    });
                } else {
                    e.insert(PointLight {
                        color: lux.color,
                        intensity,
                        radius,
                        ..default()
                    });
                }
            }
        }
    }
}
