//! Runtime selection state for the current Bevy projection.

use bevy::ecs::hierarchy::{ChildOf, Children};
use bevy::prelude::*;
use usd_bevy::{PointInstancerSelection, UsdInstanceId, UsdPrimRef};
use viewport_protocol::{ProtocolValidationError, SceneAnchor, SelectionReadModel};

/// Selected Bevy entity. This remains an internal runtime detail; the future
/// platform boundary will translate it to a stable USD scene anchor.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct SelectedPrim(pub Option<Entity>);

/// Authoritative logical selection state. The Bevy entity in [`SelectedPrim`]
/// is only the resolved primary used by internal runtime systems.
#[derive(Resource, Default, Debug, Clone)]
pub struct SelectedTargets(pub SelectionReadModel);

impl SelectedTargets {
    pub(crate) fn replace(
        &mut self,
        mut selection: SelectionReadModel,
    ) -> Result<(), ProtocolValidationError> {
        selection.canonicalize()?;
        self.0 = selection;
        Ok(())
    }

    pub(crate) fn add(
        &mut self,
        target: SceneAnchor,
        make_primary: bool,
    ) -> Result<(), ProtocolValidationError> {
        target.validate()?;
        if !self.0.targets.contains(&target) {
            self.0.targets.push(target.clone());
        }
        if make_primary {
            self.0.primary = Some(target);
        }
        self.0.canonicalize()
    }

    pub(crate) fn remove(&mut self, target: &SceneAnchor) -> Result<(), ProtocolValidationError> {
        target.validate()?;
        let removed_primary = self.0.primary.as_ref() == Some(target);
        self.0.targets.retain(|candidate| candidate != target);
        if removed_primary {
            self.0.primary = self.0.targets.first().cloned();
        }
        self.0.canonicalize()
    }
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
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn logical_selection_is_canonical_and_primary_remains_a_member() {
        let first = SceneAnchor::active_session("/World/First");
        let second = SceneAnchor::active_session("/World/Second");
        let third = SceneAnchor::active_session("/World/Third");
        let mut selection = SelectedTargets::default();

        selection
            .replace(SelectionReadModel {
                targets: vec![second.clone(), first.clone()],
                primary: Some(second.clone()),
            })
            .expect("valid selection replaces atomically");
        assert_eq!(selection.0.targets, vec![first.clone(), second.clone()]);
        assert_eq!(selection.0.primary, Some(second.clone()));

        selection
            .add(third.clone(), false)
            .expect("valid target adds to the set");
        assert_eq!(selection.0.primary, Some(second));
        selection
            .remove(&selection.0.primary.clone().unwrap())
            .expect("primary target removes");
        assert_eq!(selection.0.targets, vec![first, third]);
        assert_eq!(
            selection.0.primary,
            Some(SceneAnchor::active_session("/World/First"))
        );
    }

    #[test]
    fn logical_selection_rejects_duplicates_before_state_mutation() {
        let target = SceneAnchor::active_session("/World/Selected");
        let mut selection = SelectedTargets::default();
        assert!(
            selection
                .replace(SelectionReadModel {
                    targets: vec![target.clone(), target],
                    primary: None,
                })
                .is_err()
        );
        assert_eq!(selection.0, SelectionReadModel::default());
    }

    use bevy::asset::Assets;
    use bevy::mesh::Mesh;
    use bevy::pbr::StandardMaterial;
    use openusd::usd::Stage;
    use usd_bevy::{LiveStage, LiveStagePlugin, PointInstancerSelection, UsdPlugin};

    const FIXTURE: &str = "tests/stages/m8_point_instancer.usda";
    const INSTANCER: &str = "/World/Instances";

    #[derive(Resource, Default)]
    struct ResolvedSelection(Option<Entity>);

    fn resolve_selection_for_test(
        selection: Res<PointInstancerSelection>,
        instancers: Query<(&UsdPrimRef, &Children)>,
        instance_ids: Query<&UsdInstanceId>,
        mut resolved: ResMut<ResolvedSelection>,
    ) {
        resolved.0 = resolve_selected_instance(&selection, &instancers, &instance_ids);
    }

    fn instance_entity(world: &mut World, logical_id: i64) -> Entity {
        let mut query = world.query::<(Entity, &UsdInstanceId)>();
        query
            .iter(world)
            .find_map(|(entity, id)| (id.logical_id == logical_id).then_some(entity))
            .expect("logical instance is projected")
    }

    #[test]
    fn selected_instance_bridge_captures_identity_before_reconcile() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
        let stage =
            Stage::open(fixture.to_str().expect("fixture path is valid")).expect("fixture opens");
        let mut app = App::new();
        app.add_plugins(UsdPlugin)
            .add_plugins(LiveStagePlugin)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<SelectedPrim>()
            .init_resource::<ResolvedSelection>()
            .add_systems(
                Update,
                sync_selected_instance_identity.before(usd_bevy::LiveStageSet::Reconcile),
            )
            .add_systems(
                Update,
                resolve_selection_for_test.after(usd_bevy::LiveStageSet::Reconcile),
            );
        app.world_mut().insert_non_send(LiveStage::new(stage));
        app.update();

        let old_entity = instance_entity(app.world_mut(), 103);
        app.world_mut().resource_mut::<SelectedPrim>().0 = Some(old_entity);
        app.world()
            .get_non_send::<LiveStage>()
            .expect("live stage exists")
            .enqueue_resync(INSTANCER);

        assert_eq!(
            app.world().resource::<PointInstancerSelection>(),
            &PointInstancerSelection::default(),
            "the test must exercise the SelectedPrim bridge rather than pre-populate its output"
        );

        app.update();

        let selection = app.world().resource::<PointInstancerSelection>().clone();
        assert_eq!(selection.instancer_path.as_deref(), Some(INSTANCER));
        assert_eq!(selection.logical_id, Some(103));
        let new_entity = instance_entity(app.world_mut(), 103);
        assert_ne!(
            old_entity, new_entity,
            "resync replaces the transient child"
        );

        assert_eq!(
            app.world().resource::<ResolvedSelection>().0,
            Some(new_entity),
            "the stable selection resolves to the replacement child"
        );
    }
}
