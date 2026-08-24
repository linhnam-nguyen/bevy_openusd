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
    changed_renderables: Query<
        Entity,
        (
            With<Mesh3d>,
            Or<(
                Added<Mesh3d>,
                Changed<Mesh3d>,
                Added<GlobalTransform>,
                Changed<GlobalTransform>,
                Added<Aabb>,
                Changed<Aabb>,
                Added<UsdLocalExtent>,
                Changed<UsdLocalExtent>,
            )>,
        ),
    >,
    mut removed_meshes: RemovedComponents<Mesh3d>,
    mut removed_aabbs: RemovedComponents<Aabb>,
    mut removed_extents: RemovedComponents<UsdLocalExtent>,
    renderables: Query<(
        Option<&GlobalTransform>,
        Option<&Children>,
        Option<&Mesh3d>,
        Option<&Aabb>,
        Option<&UsdLocalExtent>,
    )>,
) {
    let targets = selection.0.targets.clone();
    let selection_changed = state.targets != targets;
    let scene_changed = state.scene_revision != scene_index.revision();
    let bounds_changed = !changed_renderables.is_empty()
        || removed_meshes.read().next().is_some()
        || removed_aabbs.read().next().is_some()
        || removed_extents.read().next().is_some();
    let enabled = settings.section_box_enabled();
    if !selection_changed && !scene_changed && !bounds_changed && !settings.is_changed() {
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
        || state.clip_planes != next_planes;
    if changed {
        state.revision = state.revision.saturating_add(1);
    }
    state.enabled = enabled;
    state.visible = next_visible;
    state.targets = targets;
    state.transform = next_transform;
    state.bounds = next_bounds;
    state.clip_planes = next_planes;
    state.scene_revision = scene_index.revision();
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
        Self {
            planes: [
                Vec4::new(1.0, 0.0, 0.0, -bounds.min.x),
                Vec4::new(-1.0, 0.0, 0.0, bounds.max.x),
                Vec4::new(0.0, 1.0, 0.0, -bounds.min.y),
                Vec4::new(0.0, -1.0, 0.0, bounds.max.y),
                Vec4::new(0.0, 0.0, 1.0, -bounds.min.z),
                Vec4::new(0.0, 0.0, -1.0, bounds.max.z),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(min: Vec3, max: Vec3) -> SectionBoxBounds {
        SectionBoxBounds { min, max }
    }

    #[test]
    fn aggregate_bounds_contain_every_selected_renderable() {
        let mut aggregate = bounds(Vec3::splat(1.0), Vec3::splat(2.0));
        aggregate.include(bounds(Vec3::splat(-4.0), Vec3::splat(-3.0)));
        aggregate.include(bounds(Vec3::splat(5.0), Vec3::splat(9.0)));

        assert!(aggregate.contains(bounds(Vec3::splat(1.0), Vec3::splat(2.0))));
        assert!(aggregate.contains(bounds(Vec3::splat(-4.0), Vec3::splat(-3.0))));
        assert!(aggregate.contains(bounds(Vec3::splat(5.0), Vec3::splat(9.0))));
        assert_eq!(aggregate.min, Vec3::splat(-4.0));
        assert_eq!(aggregate.max, Vec3::splat(9.0));
    }

    #[test]
    fn fit_produces_one_box_transform_and_six_planes() {
        let fitted = bounds(Vec3::new(-2.0, -1.0, 3.0), Vec3::new(0.0, 0.0, 6.0));
        let transform = fit_transform(fitted);
        let planes = SectionBoxClipPlanes::from_bounds(fitted);

        assert_eq!(transform.translation, Vec3::new(-1.0, -0.5, 4.5));
        assert_eq!(transform.scale, Vec3::new(2.0, 1.0, 3.0));
        assert_eq!(planes.planes.len(), 6);
    }

    #[test]
    fn empty_geometry_resets_derived_state_without_authored_geometry() {
        let mut state = SectionBoxState {
            enabled: true,
            visible: true,
            targets: vec![SceneAnchor::active_session("/World/Box")],
            transform: Transform::from_xyz(1.0, 2.0, 3.0),
            bounds: Some(bounds(Vec3::ZERO, Vec3::ONE)),
            clip_planes: SectionBoxClipPlanes::from_bounds(bounds(Vec3::ZERO, Vec3::ONE)),
            ..default()
        };
        state.reset_geometry();

        assert!(!state.visible);
        assert_eq!(state.bounds, None);
        assert_eq!(state.transform, Transform::IDENTITY);
    }
}
