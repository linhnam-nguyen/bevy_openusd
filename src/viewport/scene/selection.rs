//! Runtime selection state for the current Bevy projection.

use bevy::ecs::hierarchy::{ChildOf, Children};
use bevy::prelude::*;
use std::collections::HashSet;
use usd_bevy::{PointInstancerSelection, UsdInstanceId, UsdPrimRef};
use viewport_protocol::{
    MAX_SELECTION_TARGETS, ProtocolValidationError, SceneAnchor, SelectionReadModel,
};

/// Selected Bevy entity. This remains an internal runtime detail; the future
/// platform boundary will translate it to a stable USD scene anchor.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct SelectedPrim(pub Option<Entity>);

/// Authoritative logical selection state. The Bevy entity in [`SelectedPrim`]
/// is only the resolved primary used by internal runtime systems.
#[derive(Resource, Default, Debug, Clone)]
pub struct SelectedTargets(
    pub SelectionReadModel,
    pub(crate) u64,
    SelectionDelta,
    SelectionDelta,
);

#[derive(Default, Debug, Clone)]
pub(crate) struct SelectionDelta {
    pub(crate) added: HashSet<SceneAnchor>,
    pub(crate) removed: HashSet<SceneAnchor>,
}

impl SelectionDelta {
    fn clear(&mut self) {
        self.added.clear();
        self.removed.clear();
    }

    fn record_added(&mut self, target: SceneAnchor) {
        if !self.removed.remove(&target) {
            self.added.insert(target);
        }
    }

    fn record_removed(&mut self, target: SceneAnchor) {
        if !self.added.remove(&target) {
            self.removed.insert(target);
        }
    }
}

impl SelectedTargets {
    pub(crate) fn revision(&self) -> u64 {
        self.1
    }

    pub(crate) fn pending_delta(&self) -> &SelectionDelta {
        &self.2
    }

    pub(crate) fn clear_pending_delta(&mut self) {
        self.2.clear();
    }

    pub(crate) fn last_transaction_delta(&self) -> &SelectionDelta {
        &self.3
    }

    /// Monotonic identity for one logical selection transaction. Derived
    /// renderer projections use this instead of reconstructing the full
    /// selection value to detect stale work.
    pub(crate) fn replace(
        &mut self,
        mut selection: SelectionReadModel,
    ) -> Result<(), ProtocolValidationError> {
        selection.canonicalize()?;
        self.3.clear();
        if self.0 != selection {
            let previous_targets = self.0.targets.iter().collect::<HashSet<_>>();
            let next_targets = selection.targets.iter().collect::<HashSet<_>>();
            for target in previous_targets.difference(&next_targets) {
                self.2.record_removed((*target).clone());
            }
            for target in next_targets.difference(&previous_targets) {
                self.2.record_added((*target).clone());
            }
            self.0 = selection;
            self.1 = self.1.saturating_add(1);
        }
        Ok(())
    }

    pub(crate) fn add(
        &mut self,
        target: SceneAnchor,
        make_primary: bool,
    ) -> Result<(), ProtocolValidationError> {
        target.validate()?;
        self.3.clear();
        let before = self.0.clone();
        if !self.0.targets.contains(&target) {
            if self.0.targets.len() >= MAX_SELECTION_TARGETS {
                return Err(ProtocolValidationError::InvalidInput {
                    field: "selection.targets",
                });
            }
            self.0.targets.push(target.clone());
            self.2.record_added(target.clone());
            self.3.record_added(target.clone());
        }
        if make_primary {
            self.0.primary = Some(target);
        }
        self.0.canonicalize()?;
        if self.0 != before {
            self.1 = self.1.saturating_add(1);
        }
        Ok(())
    }

    pub(crate) fn add_many(
        &mut self,
        targets: Vec<SceneAnchor>,
        primary: Option<SceneAnchor>,
    ) -> Result<(), ProtocolValidationError> {
        validate_delta_targets(&targets)?;
        if let Some(primary) = &primary {
            primary.validate()?;
            if !targets.contains(primary) {
                return Err(ProtocolValidationError::InvalidInput {
                    field: "selection.primary",
                });
            }
        }

        let existing_targets = self.0.targets.iter().cloned().collect::<HashSet<_>>();
        let new_targets = targets
            .iter()
            .filter(|target| !existing_targets.contains(*target))
            .count();
        if self.0.targets.len() + new_targets > MAX_SELECTION_TARGETS {
            return Err(ProtocolValidationError::InvalidInput {
                field: "selection.targets",
            });
        }

        let before = self.0.clone();
        self.3.clear();
        let additions = targets
            .into_iter()
            .filter(|target| !existing_targets.contains(target))
            .collect::<Vec<_>>();
        for target in &additions {
            self.2.record_added(target.clone());
            self.3.record_added(target.clone());
        }
        self.0.targets.extend(additions);
        if let Some(primary) = primary {
            self.0.primary = Some(primary);
        }
        self.0.canonicalize()?;
        if self.0 != before {
            self.1 = self.1.saturating_add(1);
        }
        Ok(())
    }

    pub(crate) fn remove(&mut self, target: &SceneAnchor) -> Result<(), ProtocolValidationError> {
        target.validate()?;
        let before = self.0.clone();
        self.3.clear();
        let removed_primary = self.0.primary.as_ref() == Some(target);
        let removed = self.0.targets.iter().any(|candidate| candidate == target);
        self.0.targets.retain(|candidate| candidate != target);
        if removed {
            self.2.record_removed(target.clone());
            self.3.record_removed(target.clone());
        }
        if removed_primary {
            self.0.primary = self.0.targets.first().cloned();
        }
        self.0.canonicalize()?;
        if self.0 != before {
            self.1 = self.1.saturating_add(1);
        }
        Ok(())
    }

    pub(crate) fn remove_many(
        &mut self,
        targets: Vec<SceneAnchor>,
    ) -> Result<(), ProtocolValidationError> {
        validate_delta_targets(&targets)?;
        let removed_targets = targets.iter().collect::<HashSet<_>>();
        let before = self.0.clone();
        self.3.clear();
        for target in &self.0.targets {
            if removed_targets.contains(target) {
                self.2.record_removed(target.clone());
                self.3.record_removed(target.clone());
            }
        }
        self.0
            .targets
            .retain(|candidate| !removed_targets.contains(candidate));
        if self
            .0
            .primary
            .as_ref()
            .is_some_and(|primary| removed_targets.contains(primary))
        {
            self.0.primary = self.0.targets.first().cloned();
        }
        self.0.canonicalize()?;
        if self.0 != before {
            self.1 = self.1.saturating_add(1);
        }
        Ok(())
    }

    pub(crate) fn clear(&mut self) -> Result<(), ProtocolValidationError> {
        self.replace(SelectionReadModel::default())
    }
}

fn validate_delta_targets(targets: &[SceneAnchor]) -> Result<(), ProtocolValidationError> {
    if targets.len() > MAX_SELECTION_TARGETS {
        return Err(ProtocolValidationError::InvalidInput {
            field: "selection.targets",
        });
    }
    let mut seen = HashSet::with_capacity(targets.len());
    for target in targets {
        target.validate()?;
        if !seen.insert(target) {
            return Err(ProtocolValidationError::InvalidInput {
                field: "selection.targets",
            });
        }
    }
    Ok(())
}

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

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
