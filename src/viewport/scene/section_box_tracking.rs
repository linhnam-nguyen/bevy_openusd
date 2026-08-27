use std::collections::HashSet;

use bevy::camera::primitives::Aabb;
use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::UsdLocalExtent;
use viewport_protocol::SceneAnchor;

use crate::viewport::api::SceneAnchorIndex;

use super::SectionBoxTrackedRenderable;

pub(super) fn should_reconcile_section_box(
    selection_changed: bool,
    resolution_changed: bool,
    scene_revision_changed: bool,
    relevant_bounds_changed: bool,
    tracking_changed: bool,
    enabled_changed: bool,
) -> bool {
    selection_changed
        || resolution_changed
        || scene_revision_changed
        || relevant_bounds_changed
        || tracking_changed
        || enabled_changed
}

pub(super) fn reconcile_tracked_renderables(
    commands: &mut Commands,
    current: &HashSet<Entity>,
    desired: &HashSet<Entity>,
) {
    for entity in current.difference(desired) {
        commands
            .entity(*entity)
            .remove::<SectionBoxTrackedRenderable>();
    }
    for entity in desired.difference(current) {
        commands.entity(*entity).insert(SectionBoxTrackedRenderable);
    }
}

pub(in crate::viewport) fn selected_renderable_entities(
    targets: &[SceneAnchor],
    scene_index: &SceneAnchorIndex,
    renderables: &Query<(
        Option<&GlobalTransform>,
        Option<&Children>,
        Option<&Mesh3d>,
        Option<&Aabb>,
        Option<&UsdLocalExtent>,
    )>,
) -> HashSet<Entity> {
    let mut selected = HashSet::new();
    for target in targets {
        let Some(root) = scene_index.resolve(target) else {
            continue;
        };
        let mut stack = vec![root];
        let mut visited = HashSet::new();
        while let Some(entity) = stack.pop() {
            if !visited.insert(entity) {
                continue;
            }
            let Ok((_global, children, mesh, _aabb, _local_extent)) = renderables.get(entity)
            else {
                continue;
            };
            if mesh.is_some() {
                selected.insert(entity);
            }
            if let Some(children) = children {
                stack.extend(children.iter());
            }
        }
    }
    selected
}
