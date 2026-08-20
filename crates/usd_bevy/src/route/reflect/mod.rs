//! Reflect route (PLAN P1.3) — the BSN-parity core.
//!
//! Makes *any* `#[derive(Component, Reflect)] #[reflect(Component)]` type
//! authorable from USD with **no per-type Rust code**, using Bevy's type
//! registry. This is what turns `usd_bevy` from an importer into a scene
//! system: arbitrary gameplay components, authored in `.usda`, with USD
//! composition doing the field merging.
//!
//! ## Authoring convention
//!
//! A component field is authored as a custom attribute named
//! `bevy:<ShortType>:<field-path>`, where `<field-path>` uses `:` to descend
//! into nested struct fields:
//!
//! ```usda
//! def Xform "Enemy"
//! {
//!     custom double bevy:Health:current = 50
//!     custom double bevy:Health:max     = 100
//!     custom bool   bevy:Boss:enraged   = true
//! }
//! ```
//!
//! ## Sparse semantics (= BSN `TemplatePatch`, composed by USD)
//!
//! * **project**: for every `bevy:` attr on the prim, get-or-default the
//!   component and set exactly the authored fields.
//! * **patch**: set exactly the fields whose attrs changed. A USD `over` on one
//!   attribute patches one field, leaving the rest of the component intact —
//!   and because USD already composed the opinion stack, the value that lands
//!   is the LIVERPS-resolved one. When an opinion is *cleared* the field
//!   reverts to the type's `ReflectDefault`; clearing the last `bevy:` attr of
//!   a type removes the component.

mod coerce;
mod parse;

use bevy::ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy::prelude::*;
use bevy::reflect::{GetPath, std_traits::ReflectDefault};
use openusd::sdf::Value;

use super::{PrimRoute, RouteCtx};
use coerce::{set_field, try_set_handle};
use parse::{collect, resolve};

/// Maps `bevy:`-namespaced attributes onto arbitrary registered components.
pub struct ReflectRoute;

/// Per-entity bookkeeping: the `(short_type, field_paths)` the reflect route
/// has authored onto this entity. A later reconcile uses it to revert exactly
/// the fields (and components) *it* set when their USD opinions vanish — never
/// touching state a gameplay system added.
/// `(short_type, authored_field_paths)` pairs the reflect route has set.
type Record = Vec<(String, Vec<String>)>;

#[derive(Component, Clone, Default)]
struct ReflectAuthored(Record);

impl ReflectRoute {
    /// Reconcile the entity's reflect-routed components against the prim's
    /// currently-authored `bevy:` opinions — the single code path for both
    /// project and patch. It:
    /// * sets each authored field to its composed (LIVERPS-resolved) value,
    /// * reverts fields the route previously set but that are no longer
    ///   authored, back to the type default,
    /// * removes components whose last authored opinion is gone.
    ///
    /// USD has already done the opinion merging; this only projects the result.
    fn reconcile_prim(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        let now = collect(ctx);
        let prev = world
            .get::<ReflectAuthored>(entity)
            .cloned()
            .unwrap_or_default();

        // Cheap exit: a prim with no effective `bevy:` opinion that the route
        // never touched (the overwhelmingly common case on a large stage).
        let nothing_now = now
            .iter()
            .all(|(_, fs)| fs.iter().all(|(_, v)| v.is_none()));
        if nothing_now && prev.0.is_empty() {
            return;
        }

        // No type registry → nothing the reflect route can do. Warn once (only
        // when there actually *are* `bevy:` opinions to project) and bail
        // instead of panicking on a bare `World`.
        let Some(app_registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
            if !nothing_now {
                bevy::log::warn!(
                    target: "usd_bevy::route::reflect",
                    "{}: has bevy: opinions but no AppTypeRegistry — add UsdPlugin / register types",
                    ctx.prim_str()
                );
            }
            return;
        };
        let registry = app_registry.read();
        // Read the AssetServer (if any) before borrowing the entity, so
        // `Handle<T>` fields can resolve asset-path values at project time.
        let assets = world.get_resource::<AssetServer>().cloned();
        let Ok(mut ent) = world.get_entity_mut(entity) else {
            return;
        };

        let mut new_record: Record = Vec::new();

        for (ty, fields) in &now {
            // Fields with an actual authored opinion.
            let mut effective: Vec<(&String, &Value)> = fields
                .iter()
                .filter_map(|(f, v)| v.as_ref().map(|v| (f, v)))
                .collect();
            if effective.is_empty() {
                continue;
            }
            // Apply shallower paths first so an enum variant selector (`state`)
            // lands before its payload fields (`state.0`) that descend into it.
            effective.sort_by(|a, b| a.0.cmp(b.0));
            let Some(registration) = resolve(&registry, ty) else {
                bevy::log::warn!(
                    target: "usd_bevy::route::reflect",
                    "bevy:{ty}: not in the type registry — skipping (register the type + #[reflect(Component)])"
                );
                continue;
            };
            let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                bevy::log::warn!(
                    target: "usd_bevy::route::reflect",
                    "bevy:{ty}: registered but missing ReflectComponent (add #[reflect(Component)])"
                );
                continue;
            };

            // Ensure the component exists, constructing a default when absent.
            if reflect_component.reflect(&ent).is_none() {
                let Some(default) = registration.data::<ReflectDefault>() else {
                    bevy::log::warn!(
                        target: "usd_bevy::route::reflect",
                        "bevy:{ty}: no ReflectDefault and not present — cannot construct (add #[reflect(Default)])"
                    );
                    continue;
                };
                let value = default.default();
                reflect_component.insert(&mut ent, value.as_partial_reflect(), &registry);
            }
            let default_instance = registration.data::<ReflectDefault>().map(|d| d.default());

            // Set each authored field.
            let mut field_names: Vec<String> = Vec::new();
            for (field_path, v) in &effective {
                let path = format!(".{field_path}");
                if let Some(mut comp) = reflect_component.reflect_mut(&mut ent) {
                    match comp.reflect_path_mut(path.as_str()) {
                        Ok(target) => {
                            if !try_set_handle(target, v, assets.as_ref()) && !set_field(target, v)
                            {
                                bevy::log::warn!(
                                    target: "usd_bevy::route::reflect",
                                    "bevy:{ty}:{field_path}: unsupported value/type pairing — skipped"
                                );
                            }
                        }
                        Err(e) => bevy::log::warn!(
                            target: "usd_bevy::route::reflect",
                            "bevy:{ty}:{field_path}: no such field ({e})"
                        ),
                    }
                }
                field_names.push((*field_path).clone());
            }

            // Revert fields the route set previously but that are no longer
            // authored, back to the type default.
            if let Some((_, prev_fields)) = prev.0.iter().find(|(t, _)| t == ty) {
                for pf in prev_fields {
                    if field_names.iter().any(|f| f == pf) {
                        continue;
                    }
                    let path = format!(".{pf}");
                    let def_val = default_instance
                        .as_ref()
                        .and_then(|d| d.reflect_path(path.as_str()).ok())
                        .map(|f| f.to_dynamic());
                    if let Some(def_val) = def_val
                        && let Some(mut comp) = reflect_component.reflect_mut(&mut ent)
                        && let Ok(target) = comp.reflect_path_mut(path.as_str())
                    {
                        let _ = target.try_apply(&*def_val);
                    }
                }
            }

            new_record.push((ty.clone(), field_names));
        }

        // Remove components the route authored before but whose type now has no
        // effective opinion (the "cleared last field → remove component" rule).
        for (ty, _) in &prev.0 {
            if new_record.iter().any(|(t, _)| t == ty) {
                continue;
            }
            if let Some(reflect_component) =
                resolve(&registry, ty).and_then(|r| r.data::<ReflectComponent>())
            {
                reflect_component.remove(&mut ent);
            }
        }

        ent.insert(ReflectAuthored(new_record));
    }
}

impl PrimRoute for ReflectRoute {
    fn matches(&self, _ctx: &RouteCtx) -> bool {
        // Runs on every prim. The route must also see a prim whose *last*
        // `bevy:` opinion was just cleared — so it can remove the component it
        // authored. `reconcile_prim` early-outs when there is nothing to do.
        true
    }

    fn project(&self, ctx: &RouteCtx, world: &mut World, entity: Entity) {
        self.reconcile_prim(ctx, world, entity);
    }

    fn patch(&self, ctx: &RouteCtx, world: &mut World, entity: Entity, _changed: &[&str]) {
        self.reconcile_prim(ctx, world, entity);
    }
}
