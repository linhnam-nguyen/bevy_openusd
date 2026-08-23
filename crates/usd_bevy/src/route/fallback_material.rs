//! Shared renderer fallback material ownership.
//!
//! USD-authored material handles remain outside this module. The fallback is
//! one cached Bevy asset used only when a prim has no usable authored material.

use bevy::asset::Assets;
use bevy::ecs::change_detection::DetectChanges;
use bevy::ecs::resource::Resource;
use bevy::ecs::system::{Res, ResMut};
use bevy::ecs::world::World;
use bevy::pbr::StandardMaterial;
use bevy::prelude::{Color, Handle};

/// One shared material for renderable prims whose USD preview material is
/// absent or cannot be decoded.
#[derive(Resource, Clone)]
pub(crate) struct FallbackMaterial(Handle<StandardMaterial>);

/// Presentation color used only when a USD prim has no usable authored
/// material. It is intentionally separate from authored material assets.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct FallbackMaterialColor(pub Color);

impl Default for FallbackMaterialColor {
    fn default() -> Self {
        Self(Color::WHITE)
    }
}

/// Updates the shared fallback material in place, if it has already been
/// created, while retaining the same asset handle and cache ownership.
pub fn set_fallback_material_color(world: &mut World, color: Color) {
    let changed = world
        .get_resource::<FallbackMaterialColor>()
        .is_none_or(|current| current.0 != color);
    if !changed {
        return;
    }
    world.insert_resource(FallbackMaterialColor(color));

    let handle = world
        .get_resource::<FallbackMaterial>()
        .map(|fallback| fallback.0.clone());
    if let Some(handle) = handle
        && let Some(mut material) = world
            .resource_mut::<Assets<StandardMaterial>>()
            .get_mut(&handle)
    {
        material.base_color = color;
    }
}

/// Applies a changed fallback color to the existing shared asset without an
/// exclusive world access or an asset allocation on idle frames.
pub(crate) fn sync_fallback_material_color(
    color: Res<FallbackMaterialColor>,
    fallback: Option<Res<FallbackMaterial>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    if !color.is_changed() {
        return;
    }
    let Some(fallback) = fallback else {
        return;
    };
    let Some(mut materials) = materials else {
        return;
    };
    if let Some(mut material) = materials.get_mut(&fallback.0)
        && material.base_color != color.0
    {
        material.base_color = color.0;
    }
}

pub(crate) fn fallback_material(world: &mut World) -> Handle<StandardMaterial> {
    let existing = world
        .get_resource::<FallbackMaterial>()
        .map(|material| material.0.clone());
    if let Some(handle) = existing
        && world
            .resource::<Assets<StandardMaterial>>()
            .contains(&handle)
    {
        return handle;
    }
    let base_color = world
        .get_resource::<FallbackMaterialColor>()
        .map(|color| color.0)
        .unwrap_or(Color::WHITE);
    let handle = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color,
            ..Default::default()
        });
    world.insert_resource(FallbackMaterial(handle.clone()));
    handle
}

#[cfg(test)]
#[path = "fallback_material_tests.rs"]
mod tests;
