//! Runtime selection state for the current Bevy projection.

use bevy::ecs::hierarchy::{ChildOf, Children};
use bevy::prelude::*;
use usd_bevy::{PointInstancerSelection, UsdInstanceId, UsdPrimRef};

/// Selected Bevy entity. This remains an internal runtime detail; the future
/// platform boundary will translate it to a stable USD scene anchor.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct SelectedPrim(pub Option<Entity>);

/// Copies a selected instance's stable USD identity before a route is allowed
/// to replace its Bevy child entity.
pub(crate) fn sync_selected_instance_identity(
    selected: Res<SelectedPrim>,
    mut instance_selection: ResMut<PointInstancerSelection>,
    instance_ids: Query<&UsdInstanceId>,
    child_of: Query<&ChildOf>,
    prim_refs: Query<&UsdPrimRef>,
) {
    if !selected.is_changed() {
        return;
    }
    let Some(entity) = selected.0 else {
        instance_selection.clear();
        return;
    };
    let Ok(instance_id) = instance_ids.get(entity) else {
        instance_selection.clear();
        return;
    };
    let Ok(parent) = child_of.get(entity) else {
        instance_selection.clear();
        return;
    };
    let Ok(instancer) = prim_refs.get(parent.0) else {
        instance_selection.clear();
        return;
    };
    instance_selection.select(&instancer.path, instance_id.logical_id);
}

pub(crate) fn resolve_selected_instance(
    selection: &PointInstancerSelection,
    instancers: &Query<(&UsdPrimRef, &Children)>,
    instance_ids: &Query<&UsdInstanceId>,
) -> Option<Entity> {
    let (Some(path), Some(logical_id)) = (&selection.instancer_path, selection.logical_id) else {
        return None;
    };
    instancers.iter().find_map(|(prim, children)| {
        if prim.path == *path {
            children.iter().find(|child| {
                instance_ids
                    .get(*child)
                    .is_ok_and(|id| id.logical_id == logical_id)
            })
        } else {
            None
        }
    })
}
