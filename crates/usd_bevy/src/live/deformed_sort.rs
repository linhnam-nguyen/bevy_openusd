//! Opt-in sort center projection for native standard skinning.

use bevy::{
    camera::{primitives::Aabb, visibility::DynamicSkinnedMeshBounds},
    core_pipeline::core_3d::{Transparent3d, TransparentSortingInfo3d},
    pbr::Transmissive3d,
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
        render_phase::ViewSortedRenderPhases,
        sync_world::{MainEntity, MainEntityHashMap},
        view::RetainedViewEntity,
    },
};

use crate::UsdPrimRef;

/// Uses Bevy's current posed AABB for eligible standard-skinned sorted draws.
/// This is opt-in pending native image and timing qualification.
pub struct UsdDeformedSortPlugin;

impl Plugin for UsdDeformedSortPlugin {
    fn build(&self, _: &mut App) {}

    fn finish(&self, app: &mut App) {
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render
            .init_resource::<Centers>()
            .init_resource::<Overrides>()
            .add_systems(ExtractSchedule, extract_centers)
            .add_systems(
                Render,
                (
                    restore_centers
                        .after(RenderSystems::PrepareViews)
                        .before(RenderSystems::Queue),
                    apply_centers
                        .after(RenderSystems::Queue)
                        .before(RenderSystems::PhaseSort),
                ),
            );
    }
}

#[derive(Resource, Default)]
struct Centers(MainEntityHashMap<Vec3>);

type Override = (
    RetainedViewEntity,
    (Entity, MainEntity),
    TransparentSortingInfo3d,
);

#[derive(Resource, Default)]
struct Overrides {
    transparent: Vec<Override>,
    transmissive: Vec<Override>,
}

type StandardSkins<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        Ref<'static, Aabb>,
        Ref<'static, GlobalTransform>,
        &'static MeshMaterial3d<StandardMaterial>,
    ),
    (With<UsdPrimRef>, With<DynamicSkinnedMeshBounds>),
>;

fn extract_centers(
    mut centers: ResMut<Centers>,
    skins: Extract<StandardSkins>,
    materials: Extract<Option<Res<Assets<StandardMaterial>>>>,
) {
    let Some(materials) = materials.as_ref() else {
        centers.0.clear();
        return;
    };
    let eligible = |handle: &MeshMaterial3d<StandardMaterial>| {
        materials.get(&handle.0).is_some_and(|material| {
            material.specular_transmission > 0.0
                || matches!(
                    material.alpha_mode,
                    AlphaMode::Blend
                        | AlphaMode::Premultiplied
                        | AlphaMode::Add
                        | AlphaMode::Multiply
                )
        })
    };
    centers.0.retain(|entity, _| {
        skins
            .get(entity.id())
            .is_ok_and(|(_, _, _, material)| eligible(material))
    });
    for (entity, aabb, transform, material) in &skins {
        if !eligible(material) {
            continue;
        }
        let key = entity.into();
        if aabb.is_changed() || transform.is_changed() || !centers.0.contains_key(&key) {
            let center = transform.transform_point(aabb.center.into());
            if center.is_finite() {
                centers.0.insert(key, center);
            } else {
                centers.0.remove(&key);
            }
        }
    }
}

fn replace_center(
    sorting: &mut TransparentSortingInfo3d,
    center: Vec3,
) -> Option<TransparentSortingInfo3d> {
    let original = *sorting;
    if let TransparentSortingInfo3d::Sorted { mesh_center, .. } = sorting
        && *mesh_center != center
    {
        *mesh_center = center;
        return Some(original);
    }
    None
}

fn apply_centers(
    centers: Res<Centers>,
    mut overrides: ResMut<Overrides>,
    transparent: Option<ResMut<ViewSortedRenderPhases<Transparent3d>>>,
    transmissive: Option<ResMut<ViewSortedRenderPhases<Transmissive3d>>>,
) {
    if centers.0.is_empty() {
        return;
    }
    if let Some(mut phases) = transparent {
        for (&view, phase) in &mut phases.0 {
            for item in phase.items.values_mut() {
                if let Some(&center) = centers.0.get(&item.entity.1)
                    && let Some(original) = replace_center(&mut item.sorting_info, center)
                {
                    overrides.transparent.push((view, item.entity, original));
                }
            }
        }
    }
    if let Some(mut phases) = transmissive {
        for (&view, phase) in &mut phases.0 {
            for item in phase.items.values_mut() {
                if let Some(&center) = centers.0.get(&item.entity.1)
                    && let Some(original) = replace_center(&mut item.sorting_info, center)
                {
                    overrides.transmissive.push((view, item.entity, original));
                }
            }
        }
    }
}

fn restore_centers(
    mut overrides: ResMut<Overrides>,
    mut transparent: Option<ResMut<ViewSortedRenderPhases<Transparent3d>>>,
    mut transmissive: Option<ResMut<ViewSortedRenderPhases<Transmissive3d>>>,
) {
    for (view, entity, sorting) in overrides.transparent.drain(..) {
        if let Some(item) = transparent
            .as_mut()
            .and_then(|phases| phases.0.get_mut(&view))
            .and_then(|phase| phase.items.get_mut(&entity))
        {
            item.sorting_info = sorting;
        }
    }
    for (view, entity, sorting) in overrides.transmissive.drain(..) {
        if let Some(item) = transmissive
            .as_mut()
            .and_then(|phases| phases.0.get_mut(&view))
            .and_then(|phase| phase.items.get_mut(&entity))
        {
            item.sorting_info = sorting;
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/live/deformed_sort_tests.rs"]
mod tests;
