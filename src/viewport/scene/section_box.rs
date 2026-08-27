//! One renderer-owned Section Box state for the authoritative selection set.
//!
//! I1.6 owns the selection correlation, aggregate bounds, and renderer-only
//! clipping representation. Visualization, gizmo interaction, and material
//! clipping consume this state without authoring scene data.

use std::collections::HashSet;

use bevy::camera::primitives::Aabb;
use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::UsdLocalExtent;
use viewport_protocol::SceneAnchor;

use crate::viewport::api::{SceneAnchorIndex, ViewerSettingsState};
use crate::viewport::scene::selection_projection::{
    ProjectedWorldBounds, SelectedRenderableProjection,
};

#[path = "section_box_bounds.rs"]
mod section_box_bounds;
#[path = "section_box_face.rs"]
mod section_box_face;
#[path = "section_box_pose.rs"]
mod section_box_pose;
#[path = "section_box_tracking.rs"]
mod section_box_tracking;

pub(crate) use section_box_face::{SectionBoxFace, SectionBoxFaceDrag, resize_section_box_face};
pub(crate) use section_box_pose::SectionBoxPoseAuthority;
use section_box_pose::{fit_transform, next_bounds_context_generation, next_section_box_pose};
use section_box_tracking::{reconcile_tracked_renderables, should_reconcile_section_box};

pub(in crate::viewport) use section_box_bounds::aggregate_selection_bounds;
pub(in crate::viewport) use section_box_tracking::selected_renderable_entities;

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
    /// I1.6 derives the six planes from the aggregate box transform, including
    /// the user-adjusted rotation captured from the renderer gizmo.
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
    pub(crate) pose_authority: SectionBoxPoseAuthority,
    pub(crate) revision: u64,
    /// Generation of the authoritative fitted-bounds context represented by
    /// the current aggregate box. User manipulation does not change it.
    pub(crate) bounds_context_generation: u64,
    pub(crate) face_drag: Option<SectionBoxFaceDrag>,
    tracked_renderables: HashSet<Entity>,
    resolved_targets: Vec<Option<Entity>>,
    scene_revision: u64,
    projection_bounds_generation: u64,
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
            pose_authority: SectionBoxPoseAuthority::AutoFit,
            revision: 0,
            bounds_context_generation: 0,
            face_drag: None,
            tracked_renderables: HashSet::new(),
            resolved_targets: Vec::new(),
            scene_revision: 0,
            projection_bounds_generation: 0,
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
        self.pose_authority = SectionBoxPoseAuthority::AutoFit;
        self.face_drag = None;
    }
}

/// Reconciles one aggregate box after authoritative selection, scene-index, or
/// renderable-bound changes. No entity is spawned and no USD data is authored.
#[allow(clippy::type_complexity)]
pub(in crate::viewport) fn sync_section_box_state(
    settings: Res<ViewerSettingsState>,
    selection: Res<super::SelectedTargets>,
    scene_index: Res<SceneAnchorIndex>,
    projection: Option<Res<SelectedRenderableProjection>>,
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
    let projection_bounds_generation = projection
        .as_ref()
        .map_or(state.projection_bounds_generation, |projection| {
            projection.bounds_generation()
        });
    let resolved_targets = targets
        .iter()
        .map(|target| scene_index.resolve(target))
        .collect::<Vec<_>>();
    let selection_changed = state.targets != targets;
    let resolution_changed = state.resolved_targets != resolved_targets;
    let scene_revision_changed = state.scene_revision != scene_index.revision();
    let relevant_bounds_changed = if projection.is_some() {
        state.projection_bounds_generation != projection_bounds_generation
    } else {
        !changed_tracked_renderables.is_empty()
            || removed_tracked_renderables.read().next().is_some()
    };
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
        projection.as_ref().map_or_else(
            || selected_renderable_entities(&targets, &scene_index, &renderables),
            |projection| projection.renderables().clone(),
        )
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
        state.projection_bounds_generation = projection_bounds_generation;
        reconcile_tracked_renderables(
            &mut commands,
            &actual_tracked_renderables,
            &next_tracked_renderables,
        );
        return;
    }

    let next_bounds = enabled
        .then(|| {
            projection.as_ref().map_or_else(
                || aggregate_selection_bounds(&targets, &scene_index, &renderables),
                |projection| projection.aggregate_bounds().map(projected_bounds),
            )
        })
        .flatten();
    let next_visible = enabled && !targets.is_empty() && next_bounds.is_some();
    let force_auto_fit =
        selection_changed || resolution_changed || scene_revision_changed || enabled_changed;
    let next_pose = next_section_box_pose(
        state.transform,
        state.clip_planes,
        state.pose_authority,
        next_visible,
        next_bounds,
        force_auto_fit,
    );
    state.bounds_context_generation = next_bounds_context_generation(
        state.bounds_context_generation,
        state.pose_authority,
        state.bounds,
        next_visible,
        force_auto_fit,
        next_bounds,
    );

    let clipping_changed = state.enabled != enabled
        || state.visible != next_visible
        || state.targets != targets
        || state.transform != next_pose.transform
        || state.clip_planes != next_pose.clip_planes
        || state.pose_authority != next_pose.authority
        || tracked_set_changed
        || state.resolved_targets != resolved_targets;
    if clipping_changed {
        state.revision = state.revision.saturating_add(1);
    }
    state.enabled = enabled;
    state.visible = next_visible;
    state.targets = targets;
    state.transform = next_pose.transform;
    state.bounds = next_bounds;
    state.clip_planes = next_pose.clip_planes;
    state.pose_authority = next_pose.authority;
    state.tracked_renderables = next_tracked_renderables.clone();
    state.resolved_targets = resolved_targets;
    state.scene_revision = scene_index.revision();
    state.projection_bounds_generation = projection_bounds_generation;
    reconcile_tracked_renderables(
        &mut commands,
        &actual_tracked_renderables,
        &next_tracked_renderables,
    );
}

fn projected_bounds(bounds: ProjectedWorldBounds) -> SectionBoxBounds {
    SectionBoxBounds {
        min: bounds.min,
        max: bounds.max,
    }
}

impl SectionBoxClipPlanes {
    pub(super) fn from_bounds(bounds: SectionBoxBounds) -> Self {
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
