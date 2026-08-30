//! Renderer-owned temporary classification color presentation.
//!
//! The plan is a presentation input only. It is applied to shared
//! `StandardMaterial` assets and the previous route is retained in ECS
//! components so disabling the plan restores the existing authored/render-mode
//! route without touching the USD stage.

use std::collections::{HashMap, HashSet};

use bevy::ecs::hierarchy::Children;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use viewport_protocol::ColorRgb8;

use crate::viewport::api::SceneAnchorIndex;

use super::ClassificationColorPlan;
use super::section_box_clipping::SectionClipUnderlyingMaterial;
use super::selection_color::SelectionBaseMaterial;
use super::selection_outline::collect_mesh_descendants;

/// Instrumentation for the M5 idle-work and restore evidence.
#[derive(Resource, Debug, Default)]
pub(in crate::viewport) struct ClassificationColorDiagnostics {
    pub(in crate::viewport) rebuilds: u64,
    pub(in crate::viewport) rebinds: u64,
    pub(in crate::viewport) applied_entities: HashSet<Entity>,
    pub(in crate::viewport) last_generation: Option<u64>,
    pub(in crate::viewport) last_scene_revision: Option<u64>,
}

/// The route that existed before classification presentation claimed a mesh.
#[derive(Component, Debug, Clone)]
pub(in crate::viewport) struct ClassificationBaseMaterial(
    pub(in crate::viewport) Handle<StandardMaterial>,
);

/// Marks a mesh currently owned by the classification presentation layer.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::viewport) struct ClassificationColorOverride;

#[derive(Resource, Debug, Default)]
pub(in crate::viewport) struct ClassificationColorMaterialCache {
    handles: HashMap<(u8, u8, u8), Handle<StandardMaterial>>,
}

/// Rebinds the current projected meshes for one changed color plan.
///
/// The system is gated by the plan revision and scene-index revision. Shared
/// materials are cached by RGB value, and route components preserve selection,
/// render-mode, and section-box composition ownership.
#[allow(clippy::type_complexity)]
pub(in crate::viewport) fn sync_classification_color_overrides(
    plan: Res<ClassificationColorPlan>,
    scene_index: Res<SceneAnchorIndex>,
    mut diagnostics: ResMut<ClassificationColorDiagnostics>,
    mut materials_cache: ResMut<ClassificationColorMaterialCache>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_plan_revision: Local<u64>,
    mut last_scene_revision: Local<u64>,
    mut commands: Commands,
    mesh_hierarchy: Query<(Option<&Mesh3d>, Option<&Children>)>,
    mut renderables: Query<
        (
            Entity,
            Option<&mut MeshMaterial3d<StandardMaterial>>,
            Option<&mut SectionClipUnderlyingMaterial>,
            Option<&mut SelectionBaseMaterial>,
            Option<&ClassificationBaseMaterial>,
            Option<&ClassificationColorOverride>,
        ),
        With<Mesh3d>,
    >,
) {
    let scene_revision = scene_index.revision();
    if *last_plan_revision == plan.revision() && *last_scene_revision == scene_revision {
        return;
    }

    let mut desired = HashMap::<Entity, ColorRgb8>::new();
    for entry in plan.entries() {
        let roots = scene_index.resolve_all_by_prim_path(&entry.anchor.prim_path);
        for root in roots {
            let mut meshes = HashSet::new();
            collect_mesh_descendants(root, &mesh_hierarchy, &mut meshes);
            for entity in meshes {
                desired.entry(entity).or_insert(entry.color);
            }
        }
    }

    let mut work_entities = desired.keys().copied().collect::<HashSet<_>>();
    work_entities.extend(diagnostics.applied_entities.iter().copied());
    let mut work_entities = work_entities.into_iter().collect::<Vec<_>>();
    work_entities.sort_unstable();

    let mut applied_entities = HashSet::with_capacity(desired.len());
    for entity in work_entities {
        let Ok((_, mut standard, mut underlying, mut selection_base, base, marker)) =
            renderables.get_mut(entity)
        else {
            continue;
        };

        if let Some(color) = desired.get(&entity).copied() {
            let Some(base_route) = base
                .map(|material| material.0.clone())
                .or_else(|| selection_base.as_ref().map(|material| material.0.clone()))
                .or_else(|| underlying.as_ref().map(|material| material.0.clone()))
                .or_else(|| standard.as_ref().map(|material| material.0.clone()))
            else {
                continue;
            };
            let color_handle = shared_material(color, &mut materials_cache, &mut materials);
            let has_selection_base = selection_base.is_some();
            let has_underlying = underlying.is_some();
            if marker.is_none() {
                commands.entity(entity).insert((
                    ClassificationColorOverride,
                    ClassificationBaseMaterial(base_route),
                ));
            }
            if let Some(selection_base) = selection_base.as_deref_mut() {
                selection_base.0 = color_handle.clone();
            }
            if let Some(underlying) = underlying.as_deref_mut() {
                underlying.0 = color_handle.clone();
            }
            if !has_selection_base
                && !has_underlying
                && let Some(standard) = standard.as_deref_mut()
            {
                standard.0 = color_handle;
            }
            applied_entities.insert(entity);
            diagnostics.rebinds = diagnostics.rebinds.saturating_add(1);
        } else if let Some(base) = base {
            let base_handle = base.0.clone();
            if let Some(selection_base) = selection_base.as_deref_mut() {
                selection_base.0 = base_handle.clone();
            }
            if let Some(underlying) = underlying.as_deref_mut() {
                underlying.0 = base_handle.clone();
            }
            if selection_base.is_none()
                && underlying.is_none()
                && let Some(standard) = standard.as_deref_mut()
            {
                standard.0 = base_handle;
            }
            commands
                .entity(entity)
                .remove::<(ClassificationColorOverride, ClassificationBaseMaterial)>();
        }
    }

    diagnostics.applied_entities = applied_entities;
    diagnostics.rebuilds = diagnostics.rebuilds.saturating_add(1);
    diagnostics.last_generation = Some(plan.generation());
    diagnostics.last_scene_revision = Some(scene_revision);
    *last_plan_revision = plan.revision();
    *last_scene_revision = scene_revision;
}

fn shared_material(
    color: ColorRgb8,
    cache: &mut ClassificationColorMaterialCache,
    materials: &mut Assets<StandardMaterial>,
) -> Handle<StandardMaterial> {
    let key = (color.r, color.g, color.b);
    if let Some(handle) = cache.handles.get(&key) {
        return handle.clone();
    }
    let handle = materials.add(StandardMaterial {
        base_color: super::selection_color::color_from_rgb8(color),
        perceptual_roughness: 1.0,
        ..default()
    });
    cache.handles.insert(key, handle.clone());
    handle
}

#[cfg(test)]
#[path = "classification_color_tests.rs"]
mod tests;
