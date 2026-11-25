//! Renderer-local hover target tracking.
//!
//! Pointer motion remains an input detail. This module resolves only the
//! nearest changed target into a stable [`SceneAnchor`] for presentation.

use std::collections::VecDeque;

use bevy::camera::primitives::MeshAabb;
use bevy::ecs::hierarchy::ChildOf;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::pbr::MeshMaterial3d;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use bevy::render::render_resource::Face;
use usd_bevy::route::gpu_instancing::GpuPointPicker;
use usd_bevy::route::instancer::UsdOccurrenceBounds;
use viewport_protocol::{MAX_SELECTION_TARGETS, SceneAnchor, ViewportCommand};

use crate::api::ViewportCommandInbox;
use crate::viewport::navigation::input::navigation::PrimaryButtonEdge;
use crate::viewport::navigation::input::{HostUiInputCapture, ViewerInteractionGate, ViewportNavigationInput};
use crate::viewport::rendering::scene::SceneAnchorIndex;
use crate::viewport::navigation::section_box::{
    SectionBoxState, SectionClipMaterial, SectionClipProjectionState,
};

/// Current renderer-local hover target and the inputs used for its last pick.
///
/// The cache fields are deliberately renderer-local. Only `anchor` is read by
/// the presentation system, and raw pointer coordinates never cross the
/// viewport protocol a second time.
#[derive(Resource, Debug, Default, Clone)]
pub(crate) struct HoveredTarget {
    pub(crate) anchor: Option<SceneAnchor>,
    last_pointer_position: Option<Vec2>,
    last_viewport_size: Option<Vec2>,
    last_camera_local: Option<Transform>,
    last_camera_global: Option<Transform>,
    last_clip_from_view: Option<Mat4>,
    last_camera_viewport_size: Option<Vec2>,
    last_focused: Option<bool>,
    last_scene_revision: Option<u64>,
    last_visual_context: Option<usd_bevy::VisualContextId>,
    last_gpu_representation: Option<u64>,
    last_section_clip_revision: Option<u64>,
    last_click_sequence: Option<u64>,
    pending_gpu_point_pick: Option<(u64, f32)>,
    /// Every observed render-view change increments this epoch. A V0→V1→V0
    /// round trip is still stale, even when the final matrices compare equal.
    last_pick_view: Option<PickViewStamp>,
    pick_view_epoch: u64,
    pick_view_changed: bool,
}

/// The entire ray-defining context, not only the stream/scene generation.
/// A click is never reinterpreted under a later camera or presentation.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PickViewStamp {
    camera: Entity,
    world_from_view: Mat4,
    camera_local: Transform,
    clip_from_view: Mat4,
    pending_clip_from_view: Option<Mat4>,
    logical_viewport: Vec2,
    physical_viewport: Option<(UVec2, UVec2)>,
    gpu_representation: Option<u64>,
    section_box_revision: Option<u64>,
    section_clip_revision: Option<u64>,
}

fn pick_view_stamp(
    entity: Entity,
    camera: &Camera,
    camera_global: &GlobalTransform,
    camera_local: &Transform,
    projection: Option<&Projection>,
    gpu_representation: Option<u64>,
    section_box_revision: Option<u64>,
    section_clip_revision: Option<u64>,
) -> Option<PickViewStamp> {
    let viewport = camera.logical_viewport_size()?;
    Some(PickViewStamp {
        camera: entity,
        world_from_view: camera_global.to_matrix(),
        camera_local: *camera_local,
        clip_from_view: camera.clip_from_view(),
        pending_clip_from_view: projection.map(|projection| projection.get_clip_from_view()),
        logical_viewport: viewport,
        physical_viewport: camera
            .viewport
            .as_ref()
            .map(|viewport| (viewport.physical_position, viewport.physical_size)),
        gpu_representation,
        section_box_revision,
        section_clip_revision,
    })
}

/// Pure acceptance guard used by both the front-pointer hover query and the
/// final command boundary. Fail closed if a view cannot be resolved.
fn current_click_view(
    click: &QueuedPointClick,
    current: Option<PickViewStamp>,
    epoch: u64,
    generation: u64,
    scene_revision: u64,
) -> bool {
    current == Some(click.view_stamp)
        && click.view_epoch == epoch
        && click.generation == generation
        && click.scene_revision == scene_revision
}

#[derive(Clone, Copy)]
struct QueuedPointClick {
    sequence: u64,
    pointer: Vec2,
    viewport_size: Vec2,
    generation: u64,
    scene_revision: u64,
    /// Captured when the renderer first observes this release intent.
    view_stamp: PickViewStamp,
    view_epoch: u64,
    additive: bool,
}

/// A press is a gesture started in one observed view. A later release cannot
/// reinterpret its starting pointer in a different view, even for box picks.
#[derive(Clone, Copy)]
struct StampedPress {
    edge: PrimaryButtonEdge,
    view_stamp: PickViewStamp,
    view_epoch: u64,
    scene_revision: u64,
}

impl StampedPress {
    fn is_current(
        &self,
        view: PickViewStamp,
        view_epoch: u64,
        generation: u64,
        scene_revision: u64,
        input_viewport: Vec2,
    ) -> bool {
        self.view_stamp == view
            && self.view_epoch == view_epoch
            && self.edge.generation == generation
            && self.scene_revision == scene_revision
            && self.edge.viewport_size == input_viewport
    }
}

#[derive(Resource, Default)]
pub(crate) struct PointClick {
    press: Option<StampedPress>,
    pending: VecDeque<QueuedPointClick>,
    next_sequence: u64,
    input_epoch: u64,
}

impl PointClick {
    /// This must run on *every* Update, not only when the input queue has
    /// edges. Otherwise V0 press -> silent V1 camera change -> V1 release
    /// can issue a point or Ctrl/Command box selection in the wrong view.
    fn discard_stale_press(
        &mut self,
        view: PickViewStamp,
        epoch: u64,
        generation: u64,
        scene_revision: u64,
        input_viewport: Vec2,
    ) {
        if self.press.is_some_and(|press| {
            !press.is_current(view, epoch, generation, scene_revision, input_viewport)
        }) {
            self.press = None;
            bevy::log::debug!(
                target: "viewport::selection",
                "discarded held primary press after camera/view, scene, or viewport change"
            );
        }
    }
}

/// Preserve each release intent independently from the current (possibly
/// already released) button level, then wait for its own hover/pick query.
/// At most one queued tap drives hover at a time, so packed GPU readbacks
/// cannot silently turn a previous tap into a later cursor's selection.
#[allow(clippy::too_many_arguments)]
pub(crate) fn select_point_click(
    mut input: ResMut<ViewportNavigationInput>,
    hovered: Res<HoveredTarget>,
    index: Res<SceneAnchorIndex>,
    capture: Option<Res<HostUiInputCapture>>,
    inbox: Option<ResMut<ViewportCommandInbox>>,
    gate: ViewerInteractionGate,
    mut click: ResMut<PointClick>,
    cameras: Query<
        (
            Entity,
            &Camera,
            &GlobalTransform,
            &Transform,
            Option<&Projection>,
        ),
        With<Camera3d>,
    >,
    gpu_picker: Option<Res<GpuPointPicker>>,
    section_box: Option<Res<SectionBoxState>>,
    section_clip: Option<Res<SectionClipProjectionState>>,
    occurrences: Query<(
        Entity,
        &GlobalTransform,
        &UsdOccurrenceBounds,
        &InheritedVisibility,
    )>,
    selection: Res<super::SelectedTargets>,
) {
    if click.input_epoch != input.primary_epoch() {
        click.input_epoch = input.primary_epoch();
        click.press = None;
        click.pending.clear();
    }
    let (edges, overflowed) = input.take_primary_edges();
    if overflowed
        || gate.locked()
        || !input.focused
        || capture.is_some_and(|capture| capture.wants_pointer)
    {
        click.press = None;
        click.pending.clear();
        return;
    }
    let Some(mut inbox) = inbox else { return };
    // The chained hover system observes each camera/view change before this
    // consumer. Its monotonic epoch prevents V0→V1→V0 stale acceptance.
    let view = cameras
        .single()
        .ok()
        .and_then(|(entity, camera, transform, local, projection)| {
            pick_view_stamp(
                entity,
                camera,
                transform,
                local,
                projection,
                gpu_picker.as_deref().map(GpuPointPicker::view_revision),
                section_box
                    .as_ref()
                    .filter(|state| state.enabled)
                    .map(|state| state.revision),
                section_clip
                    .as_deref()
                    .map(SectionClipProjectionState::pick_revision),
            )
        });
    let Some(view) = view.filter(|view| hovered.last_pick_view == Some(*view)) else {
        click.press = None;
        click.pending.clear();
        return;
    };
    // A held press must be invalidated even if this Update contained zero
    // button edges. The queued-release guard alone does not cover box drags.
    click.discard_stale_press(
        view,
        hovered.pick_view_epoch,
        input.generation,
        index.revision(),
        input.viewport_size,
    );
    // A view transition and a release can arrive between two observations.
    // Neither host has an event-time camera token. Drop ambiguous edges
    // rather than assigning the current frame's camera to those releases.
    if hovered.pick_view_changed && !edges.is_empty() {
        click.press = None;
        bevy::log::debug!(target: "viewport::selection", "discarded input edges on a changing pick view");
    }
    let edges = if hovered.pick_view_changed {
        Vec::new()
    } else {
        edges
    };
    for edge in edges {
        if edge.pressed {
            click.press = edge.position.map(|_| StampedPress {
                edge,
                view_stamp: view,
                view_epoch: hovered.pick_view_epoch,
                scene_revision: index.revision(),
            });
            continue;
        }
        let Some(press) = click.press.take() else {
            continue;
        };
        // Release, including synchronous Ctrl/Command box selection, must
        // remain in the press's view and stream. Fail closed on a mismatch.
        if !press.is_current(
            view,
            hovered.pick_view_epoch,
            input.generation,
            index.revision(),
            input.viewport_size,
        ) || edge.generation != press.edge.generation
            || edge.viewport_size != press.edge.viewport_size
        {
            bevy::log::debug!(
                target: "viewport::selection",
                "discarded primary release without matching press view"
            );
            continue;
        }
        let (Some(start), Some(end)) = (press.edge.position, edge.position) else {
            continue;
        };
        let box_pick = press.edge.modifiers.control || press.edge.modifiers.meta;
        let moved = start.distance_squared(end) > 16.0;
        if box_pick && moved {
            if let Ok((_, camera, transform, _, _)) = cameras.single() {
                let targets = point_box_targets(
                    camera,
                    transform,
                    start,
                    end,
                    edge.viewport_size,
                    &occurrences,
                    &index,
                    &selection,
                );
                if !targets.is_empty() {
                    inbox.send(ViewportCommand::AddSelectionTargets {
                        targets,
                        primary: None,
                    });
                }
            }
            continue;
        }
        if moved {
            continue;
        }
        if click.pending.len() >= 64 {
            // No invented selection from truncated input. This is not a
            // geometric no-hit; the entire gesture queue is invalidated.
            click.press = None;
            click.pending.clear();
            bevy::log::warn!(target: "viewport::selection", "queued click capacity exceeded; cancelling clicks");
            return;
        }
        click.next_sequence = click.next_sequence.wrapping_add(1);
        let sequence = click.next_sequence;
        click.pending.push_back(QueuedPointClick {
            sequence,
            pointer: end,
            viewport_size: edge.viewport_size,
            generation: edge.generation,
            scene_revision: index.revision(),
            view_stamp: view,
            view_epoch: hovered.pick_view_epoch,
            additive: box_pick,
        });
    }
    if click.press.is_some_and(|press| {
        !press.edge.modifiers.control
            && !press.edge.modifiers.meta
            && press
                .edge
                .position
                .zip(input.pointer_position)
                .is_some_and(|(start, current)| start.distance_squared(current) > 16.0)
    }) {
        click.press = None;
    }
    let Some(next) = click.pending.front().copied() else {
        return;
    };
    if next.viewport_size != input.viewport_size
        || !current_click_view(
            &next,
            Some(view),
            hovered.pick_view_epoch,
            input.generation,
            index.revision(),
        )
    {
        // Discard this obsolete V0 intent, never repick it in V1. Keep newer
        // intents queued so they can be validated independently next update.
        click.pending.pop_front();
        bevy::log::debug!(target: "viewport::selection", sequence = next.sequence,
            "discarded click after camera/view or presentation revision changed");
        return;
    }
    // Hover updates before click in the Overlays Update chain. When a new tap
    // becomes front, its sequence forces a fresh ray/GPU request next frame.
    if hovered.last_click_sequence != Some(next.sequence)
        || hovered.last_pointer_position != Some(next.pointer)
        || hovered.last_viewport_size != Some(next.viewport_size)
        || hovered.pending_gpu_point_pick.is_some()
    {
        return;
    }
    click.pending.pop_front();
    let Some(anchor) = hovered
        .anchor
        .as_ref()
        .filter(|anchor| index.is_point_target(anchor))
    else {
        return;
    };
    let target = anchor.clone();
    inbox.send(if next.additive {
        ViewportCommand::AddSelectionTarget {
            target,
            make_primary: true,
        }
    } else {
        ViewportCommand::SelectTarget {
            target: Some(target),
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn point_box_targets(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    start: Vec2,
    end: Vec2,
    input_viewport: Vec2,
    occurrences: &Query<(
        Entity,
        &GlobalTransform,
        &UsdOccurrenceBounds,
        &InheritedVisibility,
    )>,
    index: &SceneAnchorIndex,
    selection: &super::SelectedTargets,
) -> Vec<SceneAnchor> {
    let Some(viewport) = camera.logical_viewport_size() else {
        return Vec::new();
    };
    let available = MAX_SELECTION_TARGETS.saturating_sub(selection.0.targets.len());
    if available == 0 {
        return Vec::new();
    }
    let start = map_pointer_to_camera_viewport(start, input_viewport, viewport);
    let end = map_pointer_to_camera_viewport(end, input_viewport, viewport);
    let rectangle =
        Rect::from_corners(start, end).intersect(Rect::from_corners(Vec2::ZERO, viewport));
    let previous: std::collections::HashSet<_> = selection.0.targets.iter().collect();
    let mut selected = std::collections::BTreeSet::new();
    for (entity, transform, bounds, visibility) in occurrences {
        if !visibility.get() {
            continue;
        }
        let (mut min, mut max) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for corner in 0..8 {
            let point = Vec3::new(
                if corner & 1 == 0 {
                    bounds.min.x
                } else {
                    bounds.max.x
                },
                if corner & 2 == 0 {
                    bounds.min.y
                } else {
                    bounds.max.y
                },
                if corner & 4 == 0 {
                    bounds.min.z
                } else {
                    bounds.max.z
                },
            );
            if let Ok(projected) =
                camera.world_to_viewport(camera_transform, transform.transform_point(point))
            {
                min = min.min(projected);
                max = max.max(projected);
            }
        }
        if !min.is_finite()
            || min.x > rectangle.max.x
            || max.x < rectangle.min.x
            || min.y > rectangle.max.y
            || max.y < rectangle.min.y
        {
            continue;
        }
        if let Some(anchor) = index.anchor_for(entity)
            && !previous.contains(&anchor)
        {
            selected.insert(anchor);
            if selected.len() > available {
                selected.pop_last();
            }
        }
    }
    selected.into_iter().collect()
}

/// Renderer diagnostics used by the B5 performance evidence packet.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HoverPickStats {
    pub(crate) raycasts: u64,
    pub(crate) skipped_idle_frames: u64,
}

/// Resolves the current cursor to the nearest projected mesh triangle.
///
/// Bevy's mesh ray caster uses entity AABBs only to cull candidates and then
/// intersects the actual mesh triangles. The filter excludes non-projected
/// meshes, while `anchor_for_hit` maps projected mesh children back to the
/// owning stable scene anchor.
pub(crate) fn update_hover_target(
    input: Res<ViewportNavigationInput>,
    scene_index: Res<SceneAnchorIndex>,
    live: Option<NonSend<usd_bevy::LiveStage>>,
    mut hovered: ResMut<HoveredTarget>,
    mut stats: ResMut<HoverPickStats>,
    cameras: Query<
        (
            Entity,
            &Camera,
            &GlobalTransform,
            &Transform,
            Option<&Projection>,
        ),
        With<Camera3d>,
    >,
    child_of: Query<&ChildOf>,
    mut mesh_ray_cast: MeshRayCast,
    section_view: (
        Option<Res<SectionBoxState>>,
        Option<Res<SectionClipProjectionState>>,
    ),
    clip_materials: Option<Res<Assets<SectionClipMaterial>>>,
    clipped: Query<&MeshMaterial3d<SectionClipMaterial>>,
    clipped_meshes: Query<(
        Entity,
        &Mesh3d,
        &GlobalTransform,
        &MeshMaterial3d<SectionClipMaterial>,
        &InheritedVisibility,
    )>,
    generated_source: (
        Query<(
            Entity,
            &crate::viewport::rendering::residency::GeneratedLodSource,
            &GlobalTransform,
            &InheritedVisibility,
        )>,
        Res<Assets<Mesh>>,
    ),
    gpu_picker: Option<Res<GpuPointPicker>>,
    clicks: Res<PointClick>,
    gate: ViewerInteractionGate,
) {
    if gate.locked() {
        hovered.pending_gpu_point_pick = None;
        if hovered.last_focused != Some(false) || hovered.anchor.is_some() {
            hovered.last_focused = Some(false);
            hovered.anchor = None;
        }
        return;
    }
    let (section_box, section_clip) = section_view;
    let (generated_meshes, meshes) = generated_source;
    let Ok((camera_entity, camera, camera_global, camera_local, projection)) = cameras.single()
    else {
        hovered.last_pick_view = None;
        hovered.pick_view_epoch = hovered.pick_view_epoch.wrapping_add(1);
        return;
    };

    let camera_local = *camera_local;
    let camera_global_transform = camera_global.compute_transform();
    let clip_from_view = camera.clip_from_view();
    let Some(camera_viewport_size) = camera.logical_viewport_size() else {
        return;
    };
    // Observe every view change *before* choosing a click's ray. Projection,
    // camera transform, viewport mapping, packed representation and Section Box
    // changes all invalidate an earlier click, including V0→V1→V0 sequences.
    let pick_view = pick_view_stamp(
        camera_entity,
        camera,
        camera_global,
        &camera_local,
        projection,
        gpu_picker.as_deref().map(GpuPointPicker::view_revision),
        section_box
            .as_ref()
            .filter(|state| state.enabled)
            .map(|state| state.revision),
        section_clip
            .as_deref()
            .map(SectionClipProjectionState::pick_revision),
    );
    hovered.pick_view_changed = hovered.last_pick_view != pick_view;
    if hovered.pick_view_changed {
        hovered.pick_view_epoch = hovered.pick_view_epoch.wrapping_add(1);
        hovered.last_pick_view = pick_view;
    }
    // Only a click created in this exact view may own the hover ray and GPU
    // request. Otherwise the cursor can hover normally while click is discarded.
    let front = (clicks.input_epoch == input.primary_epoch())
        .then(|| clicks.pending.front())
        .flatten()
        .filter(|click| {
            click.viewport_size == input.viewport_size
                && current_click_view(
                    click,
                    pick_view,
                    hovered.pick_view_epoch,
                    input.generation,
                    scene_index.revision(),
                )
        });
    let pick_pointer = front.map(|click| click.pointer).or(input.pointer_position);
    let pick_viewport = front.map_or(input.viewport_size, |click| click.viewport_size);
    let click_sequence = front.map(|click| click.sequence);
    let click_changed = hovered.last_click_sequence != click_sequence;
    let pointer_changed = hovered.last_pointer_position != pick_pointer;
    let viewport_changed = hovered.last_viewport_size != Some(pick_viewport);
    let camera_changed = hovered.last_camera_local != Some(camera_local)
        || hovered.last_camera_global != Some(camera_global_transform)
        || hovered.last_clip_from_view != Some(clip_from_view)
        || hovered.last_camera_viewport_size != Some(camera_viewport_size);
    let focus_changed = hovered.last_focused != Some(input.focused);
    let scene_changed = hovered.last_scene_revision != Some(scene_index.revision());
    let visual_context = live.as_deref().map(usd_bevy::LiveStage::visual_context_id);
    let context_changed = hovered.last_visual_context != visual_context;
    let gpu_representation = gpu_picker
        .as_deref()
        .map(GpuPointPicker::representation_revision);
    let gpu_changed = hovered.last_gpu_representation != gpu_representation;
    let section_clip_revision = section_clip
        .as_deref()
        .map(SectionClipProjectionState::pick_revision);
    let section_clip_changed = hovered.last_section_clip_revision != section_clip_revision;

    if !pointer_changed
        && !viewport_changed
        && !camera_changed
        && !focus_changed
        && !scene_changed
        && !context_changed
        && !gpu_changed
        && !section_clip_changed
        && !click_changed
    {
        if let Some((sequence, entity_depth)) = hovered.pending_gpu_point_pick
            && let Some(result) = gpu_picker
                .as_deref()
                .and_then(|picker| picker.take_result(sequence))
        {
            hovered.pending_gpu_point_pick = None;
            match result {
                Ok(result) => {
                    if let Some(hit) = result.hit
                        && hit.distance < entity_depth
                        && let Some(anchor) =
                            scene_index.point_anchor(&hit.instancer, hit.logical_id)
                    {
                        hovered.anchor = Some(anchor);
                    }
                }
                Err(error) => bevy::log::warn!(
                    target: "viewport::selection", error = ?error,
                    "packed point-pick unavailable; not treating it as a geometric miss"
                ),
            }
        }
        stats.skipped_idle_frames = stats.skipped_idle_frames.saturating_add(1);
        return;
    }

    hovered.last_pointer_position = pick_pointer;
    hovered.last_viewport_size = Some(pick_viewport);
    hovered.last_click_sequence = click_sequence;
    hovered.last_camera_local = Some(camera_local);
    hovered.last_camera_global = Some(camera_global_transform);
    hovered.last_clip_from_view = Some(clip_from_view);
    hovered.last_camera_viewport_size = Some(camera_viewport_size);
    hovered.last_focused = Some(input.focused);
    hovered.last_scene_revision = Some(scene_index.revision());
    hovered.last_visual_context = visual_context;
    hovered.last_gpu_representation = gpu_representation;
    hovered.last_section_clip_revision = section_clip_revision;
    hovered.pending_gpu_point_pick = None;
    stats.raycasts = stats.raycasts.saturating_add(1);

    let next = if !input.focused {
        None
    } else if let Some(pointer) = pick_pointer {
        let pointer = map_pointer_to_camera_viewport(pointer, pick_viewport, camera_viewport_size);
        (pointer.cmpge(Vec2::ZERO).all() && pointer.cmple(camera_viewport_size).all())
            .then(|| camera.viewport_to_world(camera_global, pointer).ok())
            .flatten()
            .and_then(|ray| {
                // Bevy MeshRayCast reports only the first intersection for
                // each entity. Exclude section-clipped entities from that
                // query, then traverse *all* their triangles below; filtering
                // the first reported hit would hide a visible rear surface.
                let filter = |entity| {
                    clipped.get(entity).is_err()
                        && generated_meshes.get(entity).is_err()
                        && anchor_for_hit(entity, &scene_index, &child_of).is_some()
                };
                let settings = MeshRayCastSettings::default().with_filter(&filter);
                let ordinary =
                    mesh_ray_cast
                        .cast_ray(ray, &settings)
                        .iter()
                        .find_map(|(entity, hit)| {
                            anchor_for_hit(*entity, &scene_index, &child_of)
                                .map(|anchor| (anchor, hit.distance))
                        });
                let mut hit = ordinary;
                for (entity, full, transform, visibility) in &generated_meshes {
                    if !visibility.get() {
                        continue;
                    }
                    let Some(anchor) = anchor_for_hit(entity, &scene_index, &child_of) else {
                        continue;
                    };
                    let Some(mesh) = meshes.get(&full.0) else {
                        continue;
                    };
                    if let Ok(Some(distance)) = nearest_section_clip_triangle(
                        ray,
                        mesh,
                        transform,
                        Some(bevy::render::render_resource::Face::Back),
                        |_| true,
                    ) && hit.as_ref().is_none_or(|(_, best)| distance < *best)
                    {
                        hit = Some((anchor, distance));
                    }
                }
                for (entity, mesh3d, transform, material_handle, visibility) in &clipped_meshes {
                    if !visibility.get() {
                        continue;
                    }
                    let Some(anchor) = anchor_for_hit(entity, &scene_index, &child_of) else {
                        continue;
                    };
                    let (Some(material), Some(mesh)) = (
                        clip_materials
                            .as_ref()
                            .and_then(|assets| assets.get(&material_handle.0)),
                        meshes.get(&mesh3d.0),
                    ) else {
                        bevy::log::warn!(target: "viewport::selection", entity = ?entity,
                            "section-clipped mesh or material unavailable for exact picking");
                        continue;
                    };
                    match nearest_section_clip_triangle(
                        ray,
                        mesh,
                        transform,
                        material.base.cull_mode,
                        |point| material.extension.keeps_world_point(point),
                    ) {
                        Ok(Some(distance))
                            if hit.as_ref().is_none_or(|(_, best)| distance < *best) =>
                        {
                            hit = Some((anchor, distance));
                        }
                        Ok(_) => {}
                        Err(error) => bevy::log::warn!(
                            target: "viewport::selection", entity = ?entity, error = ?error,
                            "section-clipped mesh cannot be picked exactly"
                        ),
                    }
                }
                if let Some(picker) = gpu_picker.as_deref() {
                    let depth = hit.as_ref().map_or(f32::INFINITY, |(_, depth)| *depth);
                    hovered.pending_gpu_point_pick = Some((picker.request(ray), depth));
                }
                hit.map(|(anchor, _)| anchor)
            })
    } else {
        None
    };

    if hovered.anchor != next {
        hovered.anchor = next;
    }
}

/// Clip-aware ray intersection for one projected mesh. Unlike MeshRayCast,
/// this traverses later triangles of the *same* mesh after a clipped hit.
/// Return world-space ray distance so ordinary, clipped and GPU hits share
/// the same nearest-depth contract. Never modify the authored USD mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClipPickError {
    UnsupportedTopology,
    MissingPositions,
    InvalidTransform,
}

fn nearest_section_clip_triangle(
    ray: Ray3d,
    mesh: &Mesh,
    transform: &GlobalTransform,
    cull_mode: Option<Face>,
    keeps_world_point: impl Fn(Vec3) -> bool,
) -> Result<Option<f32>, ClipPickError> {
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return Err(ClipPickError::UnsupportedTopology);
    }
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return Err(ClipPickError::MissingPositions);
    };
    let world_from_mesh = transform.to_matrix();
    if !world_from_mesh.is_finite() || world_from_mesh.determinant().abs() < 1e-12 {
        return Err(ClipPickError::InvalidTransform);
    }
    let mesh_from_world = world_from_mesh.inverse();
    let local_origin = mesh_from_world.transform_point3(ray.origin);
    let local_direction = mesh_from_world.transform_vector3(ray.direction.as_vec3());
    if !local_origin.is_finite() || !local_direction.is_finite() {
        return Err(ClipPickError::InvalidTransform);
    }
    if let Some(bounds) = mesh.get_aabb() {
        let min: Vec3 = bounds.min().into();
        let max: Vec3 = bounds.max().into();
        if !ray_overlaps_aabb(local_origin, local_direction, min, max) {
            return Ok(None);
        }
    }
    let indices = mesh.indices();
    let vertex_count = indices.map_or(positions.len(), Indices::len);
    let mut nearest: Option<f32> = None;
    let winding = world_from_mesh.determinant().signum();
    for start in (0..vertex_count.saturating_sub(2)).step_by(3) {
        let vertex = |offset: usize| -> Option<Vec3> {
            let index = match indices {
                Some(Indices::U16(values)) => usize::from(*values.get(start + offset)?),
                Some(Indices::U32(values)) => *values.get(start + offset)? as usize,
                None => start + offset,
            };
            positions.get(index).map(|v| Vec3::from_array(*v))
        };
        let (Some(a), Some(b), Some(c)) = (vertex(0), vertex(1), vertex(2)) else {
            // An invalid indexed triangle cannot produce a visible raster
            // hit, so continue to the next independent triangle.
            continue;
        };
        let Some(local_t) =
            ray_triangle_parameter(local_origin, local_direction, a, b, c, cull_mode, winding)
        else {
            continue;
        };
        let hit_world = world_from_mesh.transform_point3(local_origin + local_direction * local_t);
        if !keeps_world_point(hit_world) {
            continue;
        }
        let distance = (hit_world - ray.origin).dot(ray.direction.as_vec3());
        if distance.is_finite() && distance > 0.0 && nearest.is_none_or(|best| distance < best) {
            nearest = Some(distance);
        }
    }
    Ok(nearest)
}

/// Möller–Trumbore, with winding corrected for reflected mesh transforms.
fn ray_triangle_parameter(
    origin: Vec3,
    direction: Vec3,
    a: Vec3,
    b: Vec3,
    c: Vec3,
    cull: Option<Face>,
    winding: f32,
) -> Option<f32> {
    let ab = b - a;
    let ac = c - a;
    let p = direction.cross(ac);
    let det = ab.dot(p);
    let visible_face = match cull {
        Some(Face::Back) => det * winding > 1e-8,
        Some(Face::Front) => det * winding < -1e-8,
        None => det.abs() > 1e-8,
    };
    if !visible_face {
        return None;
    }
    let inv_det = 1.0 / det;
    let delta = origin - a;
    let u = delta.dot(p) * inv_det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = delta.cross(ab);
    let v = direction.dot(q) * inv_det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = ac.dot(q) * inv_det;
    (t.is_finite() && t > 0.0).then_some(t)
}

fn ray_overlaps_aabb(origin: Vec3, direction: Vec3, min: Vec3, max: Vec3) -> bool {
    let (mut enter, mut leave) = (0.0_f32, f32::INFINITY);
    for axis in 0..3 {
        let o = origin[axis];
        let d = direction[axis];
        if d.abs() < 1e-20 {
            if o < min[axis] || o > max[axis] {
                return false;
            }
        } else {
            let a = (min[axis] - o) / d;
            let b = (max[axis] - o) / d;
            enter = enter.max(a.min(b));
            leave = leave.min(a.max(b));
            if leave < enter {
                return false;
            }
        }
    }
    true
}

/// Maps CSS or native cursor coordinates to the displayed camera rectangle,
/// leaving letterbox/pillarbox coordinates outside the render target.
fn map_pointer_to_camera_viewport(
    pointer: Vec2,
    input_viewport_size: Vec2,
    camera_viewport_size: Vec2,
) -> Vec2 {
    let input_viewport_size = input_viewport_size.max(Vec2::splat(f32::EPSILON));
    let scale = (input_viewport_size / camera_viewport_size).min_element();
    let origin = (input_viewport_size - camera_viewport_size * scale) * 0.5;
    (pointer - origin) / scale
}

/// Resolves a mesh hit on a projected child entity to its owning prim.
fn anchor_for_hit(
    mut entity: Entity,
    scene_index: &SceneAnchorIndex,
    child_of: &Query<&ChildOf>,
) -> Option<SceneAnchor> {
    // The hierarchy is authored by scene projection and cannot contain a
    // cycle. The bound keeps malformed external ECS state from spinning the
    // hover system forever.
    for _ in 0..256 {
        if let Some(anchor) = scene_index.anchor_for(entity) {
            return Some(anchor);
        }
        entity = child_of.get(entity).ok()?.parent();
    }
    None
}

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/selection/hover_tests.rs"]
mod tests;
