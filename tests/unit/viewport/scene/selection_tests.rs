use super::*;
use bevy::asset::Assets;
use bevy::mesh::Mesh;
use bevy::pbr::StandardMaterial;
use openusd::usd::Stage;
use std::path::PathBuf;
use usd_bevy::{LiveStage, LiveStagePlugin, PointInstancerSelection, UsdPlugin};

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
    assert_eq!(selection.revision(), 1);
    assert_eq!(selection.0.targets, vec![first.clone(), second.clone()]);
    assert_eq!(selection.0.primary, Some(second.clone()));

    selection
        .add(third.clone(), false)
        .expect("valid target adds to the set");
    assert_eq!(selection.revision(), 2);
    assert_eq!(selection.0.primary, Some(second));
    selection
        .add(third.clone(), false)
        .expect("adding an existing non-primary target is a no-op");
    assert_eq!(selection.revision(), 2);
    selection
        .remove(&selection.0.primary.clone().unwrap())
        .expect("primary target removes");
    assert_eq!(selection.revision(), 3);
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
    assert_eq!(selection.revision(), 0);
}

#[test]
fn large_selection_transactions_advance_the_authority_once_each() {
    let mut selection = SelectedTargets::default();
    let initial_targets = (0..1_000)
        .map(|index| SceneAnchor::active_session(format!("/World/Initial{index:04}")))
        .collect::<Vec<_>>();
    selection
        .replace(SelectionReadModel {
            primary: initial_targets.first().cloned(),
            targets: initial_targets,
        })
        .expect("1,000-target replacement must be valid");
    assert_eq!(selection.revision(), 1);
    assert_eq!(selection.0.targets.len(), 1_000);

    let additions = (0..100)
        .map(|index| SceneAnchor::active_session(format!("/World/Added{index:04}")))
        .collect::<Vec<_>>();
    selection
        .add_many(additions, None)
        .expect("100-target addition must be valid");
    assert_eq!(selection.revision(), 2);
    assert_eq!(selection.0.targets.len(), 1_100);

    let removals = (0..100)
        .map(|index| SceneAnchor::active_session(format!("/World/Initial{index:04}")))
        .collect::<Vec<_>>();
    selection
        .remove_many(removals)
        .expect("100-target removal must be valid");
    assert_eq!(selection.revision(), 3);
    assert_eq!(selection.0.targets.len(), 1_000);
    assert_eq!(
        selection.0.primary,
        Some(SceneAnchor::active_session("/World/Added0000"))
    );
}

#[test]
fn pending_selection_delta_coalesces_until_projection_consumes_it() {
    let first = SceneAnchor::active_session("/World/First");
    let second = SceneAnchor::active_session("/World/Second");
    let mut selection = SelectedTargets::default();

    selection
        .add_many(vec![first.clone(), second.clone()], None)
        .expect("valid targets add");
    selection
        .remove_many(vec![first])
        .expect("valid target removal");

    assert_eq!(selection.pending_delta().added.len(), 1);
    assert!(selection.pending_delta().added.contains(&second));
    assert!(selection.pending_delta().removed.is_empty());

    selection.clear_pending_delta();
    assert!(selection.pending_delta().added.is_empty());
    assert!(selection.pending_delta().removed.is_empty());
}

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
