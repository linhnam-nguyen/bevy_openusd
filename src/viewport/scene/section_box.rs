//! One renderer-owned Section Box state for the authoritative selection set.
//!
//! B6.1 owns only the selection correlation, aggregate bounds, and derived
//! clipping representation. Visualization, gizmo interaction, and material
//! clipping remain later checkpoints.

use std::collections::HashSet;

use bevy::camera::primitives::Aabb;
use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::UsdLocalExtent;
use viewport_protocol::SceneAnchor;

use crate::viewport::api::{SceneAnchorIndex, ViewerSettingsState};

#[path = "section_box_tracking.rs"]
mod section_box_tracking;

use section_box_tracking::{
    reconcile_tracked_renderables, selected_renderable_entities, should_reconcile_section_box,
};

const MIN_BOX_SIZE: f32 = 0.0001;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SectionBoxBounds {
    pub(crate) min: Vec3,
    pub(crate) max: Vec3,
}

impl SectionBoxBounds {
    fn include(&mut self, other: Self) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }

    fn contains(&self, other: Self) -> bool {
        self.min.cmple(other.min).all() && self.max.cmpge(other.max).all()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SectionBoxClipPlanes {
    /// Planes use `dot(normal, world_position) + offset >= 0` as the kept side.
    /// B6.1 fits an axis-aligned box; later rotation updates will derive the
    /// same six planes from the aggregate box transform.
    pub(crate) planes: [Vec4; 6],
}

/// Marks only the currently selected renderable descendants whose bound
/// changes can invalidate the aggregate Section Box.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::viewport) struct SectionBoxTrackedRenderable;

impl Default for SectionBoxClipPlanes {
    fn default() -> Self {
        Self {
            planes: [Vec4::ZERO; 6],
        }
    }
}

#[derive(Resource, Debug, Clone, PartialEq)]
pub(crate) struct SectionBoxState {
    /// The authoritative user setting, retained even when no target is visible.
    pub(crate) enabled: bool,
    /// Effective visibility is false when the setting is enabled but the
    /// authoritative selection has no resolved renderable bounds.
    pub(crate) visible: bool,
    pub(crate) targets: Vec<SceneAnchor>,
    pub(crate) transform: Transform,
    pub(crate) bounds: Option<SectionBoxBounds>,
    pub(crate) clip_planes: SectionBoxClipPlanes,
    pub(crate) revision: u64,
    tracked_renderables: HashSet<Entity>,
    resolved_targets: Vec<Option<Entity>>,
    scene_revision: u64,
}

impl Default for SectionBoxState {
    fn default() -> Self {
        Self {
            enabled: false,
            visible: false,
            targets: Vec::new(),
            transform: Transform::IDENTITY,
            bounds: None,
            clip_planes: SectionBoxClipPlanes::default(),
            revision: 0,
            tracked_renderables: HashSet::new(),
            resolved_targets: Vec::new(),
            scene_revision: 0,
        }
    }
}

impl SectionBoxState {
    /// Clears the derived box while preserving the authoritative enabled flag.
    /// The next selection or scene update will refit it from current bounds.
    pub(crate) fn reset_geometry(&mut self) {
        self.visible = false;
        self.transform = Transform::IDENTITY;
        self.bounds = None;
        self.clip_planes = SectionBoxClipPlanes::default();
    }
}

/// Reconciles one aggregate box after authoritative selection, scene-index, or
/// renderable-bound changes. No entity is spawned and no USD data is authored.
#[allow(clippy::type_complexity)]
pub(in crate::viewport) fn sync_section_box_state(
    settings: Res<ViewerSettingsState>,
    selection: Res<super::SelectedTargets>,
    scene_index: Res<SceneAnchorIndex>,
    mut state: ResMut<SectionBoxState>,
    changed_tracked_renderables: Query<
        Entity,
        (
            With<SectionBoxTrackedRenderable>,
            Or<(
                Added<Mesh3d>,
                Changed<Mesh3d>,
                Added<GlobalTransform>,
                Changed<GlobalTransform>,
                Added<Aabb>,
                Changed<Aabb>,
                Added<UsdLocalExtent>,
                Changed<UsdLocalExtent>,
                Added<MeshMaterial3d<StandardMaterial>>,
                Changed<MeshMaterial3d<StandardMaterial>>,
            )>,
        ),
    >,
    mut removed_tracked_renderables: RemovedComponents<SectionBoxTrackedRenderable>,
    tracked_entities: Query<Entity, With<SectionBoxTrackedRenderable>>,
    mut commands: Commands,
    renderables: Query<(
        Option<&GlobalTransform>,
        Option<&Children>,
        Option<&Mesh3d>,
        Option<&Aabb>,
        Option<&UsdLocalExtent>,
    )>,
) {
    let targets = selection.0.targets.clone();
    let resolved_targets = targets
        .iter()
        .map(|target| scene_index.resolve(target))
        .collect::<Vec<_>>();
    let selection_changed = state.targets != targets;
    let resolution_changed = state.resolved_targets != resolved_targets;
    let scene_revision_changed = state.scene_revision != scene_index.revision();
    let relevant_bounds_changed = !changed_tracked_renderables.is_empty()
        || removed_tracked_renderables.read().next().is_some();
    let actual_tracked_renderables = tracked_entities.iter().collect::<HashSet<_>>();
    let tracking_changed = actual_tracked_renderables != state.tracked_renderables;
    let enabled = settings.section_box_enabled();
    let enabled_changed = state.enabled != enabled;
    if !should_reconcile_section_box(
        selection_changed,
        resolution_changed,
        scene_revision_changed,
        relevant_bounds_changed,
        tracking_changed,
        enabled_changed,
    ) {
        return;
    }

    let next_tracked_renderables = if enabled {
        selected_renderable_entities(&targets, &scene_index, &renderables)
    } else {
        HashSet::new()
    };
    let tracked_set_changed = state.tracked_renderables != next_tracked_renderables;
    if !selection_changed
        && !resolution_changed
        && !relevant_bounds_changed
        && !tracked_set_changed
        && !enabled_changed
    {
        state.resolved_targets = resolved_targets;
        state.scene_revision = scene_index.revision();
        reconcile_tracked_renderables(
            &mut commands,
            &actual_tracked_renderables,
            &next_tracked_renderables,
        );
        return;
    }

    let next_bounds = enabled
        .then(|| aggregate_selection_bounds(&targets, &scene_index, &renderables))
        .flatten();
    let next_visible = enabled && !targets.is_empty() && next_bounds.is_some();
    let next_transform = next_bounds
        .map(fit_transform)
        .unwrap_or(Transform::IDENTITY);
    let next_planes = next_bounds
        .map(SectionBoxClipPlanes::from_bounds)
        .unwrap_or_default();

    let changed = state.enabled != enabled
        || state.visible != next_visible
        || state.targets != targets
        || state.bounds != next_bounds
        || state.transform != next_transform
        || state.clip_planes != next_planes
        || tracked_set_changed
        || state.resolved_targets != resolved_targets;
    if changed {
        state.revision = state.revision.saturating_add(1);
    }
    state.enabled = enabled;
    state.visible = next_visible;
    state.targets = targets;
    state.transform = next_transform;
    state.bounds = next_bounds;
    state.clip_planes = next_planes;
    state.tracked_renderables = next_tracked_renderables.clone();
    state.resolved_targets = resolved_targets;
    state.scene_revision = scene_index.revision();
    reconcile_tracked_renderables(
        &mut commands,
        &actual_tracked_renderables,
        &next_tracked_renderables,
    );
}

fn aggregate_selection_bounds(
    targets: &[SceneAnchor],
    scene_index: &SceneAnchorIndex,
    renderables: &Query<(
        Option<&GlobalTransform>,
        Option<&Children>,
        Option<&Mesh3d>,
        Option<&Aabb>,
        Option<&UsdLocalExtent>,
    )>,
) -> Option<SectionBoxBounds> {
    let mut aggregate: Option<SectionBoxBounds> = None;
    for target in targets {
        let Some(root) = scene_index.resolve(target) else {
            continue;
        };
        let Some(target_bounds) = target_bounds(root, renderables) else {
            continue;
        };
        if let Some(current) = &mut aggregate {
            current.include(target_bounds);
        } else {
            aggregate = Some(target_bounds);
        }
    }
    aggregate
}

fn target_bounds(
    root: Entity,
    renderables: &Query<(
        Option<&GlobalTransform>,
        Option<&Children>,
        Option<&Mesh3d>,
        Option<&Aabb>,
        Option<&UsdLocalExtent>,
    )>,
) -> Option<SectionBoxBounds> {
    let mut stack = vec![root];
    let mut visited = HashSet::new();
    let mut aggregate: Option<SectionBoxBounds> = None;
    while let Some(entity) = stack.pop() {
        if !visited.insert(entity) {
            continue;
        }
        let Ok((global, children, mesh, aabb, local_extent)) = renderables.get(entity) else {
            continue;
        };
        if let (Some(global), Some(_mesh)) = (global, mesh) {
            let local_bounds = local_extent
                .map(|extent| SectionBoxBounds {
                    min: Vec3::from_array(extent.min),
                    max: Vec3::from_array(extent.max),
                })
                .or_else(|| {
                    aabb.map(|aabb| {
                        let center = Vec3::from(aabb.center);
                        let half_extents = Vec3::from(aabb.half_extents);
                        SectionBoxBounds {
                            min: center - half_extents,
                            max: center + half_extents,
                        }
                    })
                });
            if let Some(local_bounds) = local_bounds {
                let world_bounds = transform_bounds(local_bounds, global.to_matrix());
                if let Some(current) = &mut aggregate {
                    current.include(world_bounds);
                } else {
                    aggregate = Some(world_bounds);
                }
            }
        }
        if let Some(children) = children {
            stack.extend(children.iter());
        }
    }
    aggregate
}

fn transform_bounds(bounds: SectionBoxBounds, matrix: Mat4) -> SectionBoxBounds {
    let mut transformed = SectionBoxBounds {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };
    for index in 0..8 {
        let corner = Vec3::new(
            if index & 1 == 0 {
                bounds.min.x
            } else {
                bounds.max.x
            },
            if index & 2 == 0 {
                bounds.min.y
            } else {
                bounds.max.y
            },
            if index & 4 == 0 {
                bounds.min.z
            } else {
                bounds.max.z
            },
        );
        let world = matrix.transform_point3(corner);
        transformed.min = transformed.min.min(world);
        transformed.max = transformed.max.max(world);
    }
    transformed
}

fn fit_transform(bounds: SectionBoxBounds) -> Transform {
    Transform {
        translation: (bounds.min + bounds.max) * 0.5,
        scale: (bounds.max - bounds.min).max(Vec3::splat(MIN_BOX_SIZE)),
        ..Transform::IDENTITY
    }
}

impl SectionBoxClipPlanes {
    fn from_bounds(bounds: SectionBoxBounds) -> Self {
        Self::from_transform(fit_transform(bounds))
    }

    pub(crate) fn from_transform(transform: Transform) -> Self {
        let half_extents = transform.scale.abs() * 0.5;
        let axes = [
            transform.rotation * Vec3::X,
            transform.rotation * Vec3::Y,
            transform.rotation * Vec3::Z,
        ];
        let center = transform.translation;

        let plane =
            |normal: Vec3, point: Vec3| Vec4::new(normal.x, normal.y, normal.z, -normal.dot(point));

        Self {
            planes: [
                plane(axes[0], center - axes[0] * half_extents.x),
                plane(-axes[0], center + axes[0] * half_extents.x),
                plane(axes[1], center - axes[1] * half_extents.y),
                plane(-axes[1], center + axes[1] * half_extents.y),
                plane(axes[2], center - axes[2] * half_extents.z),
                plane(-axes[2], center + axes[2] * half_extents.z),
            ],
        }
    }
}

#[cfg(test)]
#[path = "section_box_tests.rs"]
mod tests;
