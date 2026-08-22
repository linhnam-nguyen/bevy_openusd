use bevy::prelude::*;
use usd_bevy::UsdPrimRef;
use viewport_protocol::*;

use super::support::command_test_app;
use crate::viewport::api::ViewportReadModelState;
use crate::viewport::api::bridge::commands::apply_viewport_commands;
use crate::viewport::api::scene_index::refresh_scene_anchor_index;
use crate::viewport::api::{SceneAnchorIndex, ViewportCommandInbox, ViewportEventOutbox};
use crate::viewport::scene::{SelectedPrim, SelectedTargets};
use crate::viewport::session::Spawned;

const FIRST: &str = "/World/First";
const SECOND: &str = "/World/Second";
const THIRD: &str = "/World/Third";

fn selection_test_app() -> App {
    let mut app = command_test_app();
    app.add_systems(
        Update,
        refresh_scene_anchor_index.before(apply_viewport_commands),
    );
    for path in [FIRST, SECOND, THIRD] {
        app.world_mut().spawn(UsdPrimRef::new(path));
    }
    app.world_mut().resource_mut::<Spawned>().0 = true;
    app.update();
    app
}

fn anchor(path: &str) -> SceneAnchor {
    SceneAnchor::active_session(path)
}

fn next_event(app: &mut App) -> ViewportEventEnvelope {
    app.world_mut()
        .resource_mut::<ViewportEventOutbox>()
        .pop()
        .expect("selection command must publish an event")
}

#[test]
fn select_target_supports_zero_one_and_many_authoritative_targets() {
    let mut app = selection_test_app();
    app.world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::SelectTarget { target: None });
    app.update();
    let event = next_event(&mut app);
    assert!(matches!(
        event.event,
        ViewportEvent::SelectionChanged { .. }
    ));
    assert!(
        app.world()
            .resource::<SelectedTargets>()
            .0
            .targets
            .is_empty()
    );
    assert!(app.world().resource::<SelectedPrim>().0.is_none());

    app.world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::SelectTarget {
            target: Some(anchor(FIRST)),
        });
    app.update();
    let event = next_event(&mut app);
    assert!(matches!(
        event.event,
        ViewportEvent::SelectionChanged { .. }
    ));
    assert_eq!(
        app.world().resource::<SelectedTargets>().0,
        SelectionReadModel::from_legacy_target(Some(anchor(FIRST)))
    );
    assert!(app.world().resource::<SelectedPrim>().0.is_some());

    let targets = vec![anchor(THIRD), anchor(FIRST), anchor(SECOND)];
    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::ReplaceSelection {
            targets,
            primary: Some(anchor(SECOND)),
        },
    );
    app.update();
    let event = next_event(&mut app);
    assert!(matches!(
        event.event,
        ViewportEvent::SelectionChanged { .. }
    ));
    let selection = &app.world().resource::<SelectedTargets>().0;
    assert_eq!(
        selection.targets,
        vec![anchor(FIRST), anchor(SECOND), anchor(THIRD)]
    );
    assert_eq!(selection.primary, Some(anchor(SECOND)));
    assert_eq!(
        app.world().resource::<SelectedPrim>().0,
        app.world()
            .resource::<SceneAnchorIndex>()
            .resolve(&anchor(SECOND))
    );
}

#[test]
fn unresolved_replace_is_atomic_and_add_remove_preserve_primary_invariants() {
    let mut app = selection_test_app();
    app.world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::SelectTarget {
            target: Some(anchor(FIRST)),
        });
    app.update();
    let _ = next_event(&mut app);
    let before = app.world().resource::<SelectedTargets>().0.clone();
    let before_primary = app.world().resource::<SelectedPrim>().0;

    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::ReplaceSelection {
            targets: vec![anchor(FIRST), anchor("/World/Missing")],
            primary: Some(anchor(FIRST)),
        },
    );
    app.update();
    assert!(matches!(
        next_event(&mut app).event,
        ViewportEvent::CommandRejected { .. }
    ));
    assert_eq!(app.world().resource::<SelectedTargets>().0, before);
    assert_eq!(app.world().resource::<SelectedPrim>().0, before_primary);

    for (target, make_primary) in [(SECOND, true), (SECOND, false)] {
        app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::AddSelectionTarget {
                target: anchor(target),
                make_primary,
            },
        );
        app.update();
        assert!(matches!(
            next_event(&mut app).event,
            ViewportEvent::SelectionChanged { .. }
        ));
    }
    assert_eq!(app.world().resource::<SelectedTargets>().0.targets.len(), 2);
    assert_eq!(
        app.world().resource::<SelectedTargets>().0.primary,
        Some(anchor(SECOND))
    );

    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::RemoveSelectionTarget {
            target: anchor(SECOND),
        },
    );
    app.update();
    assert!(matches!(
        next_event(&mut app).event,
        ViewportEvent::SelectionChanged { .. }
    ));
    assert_eq!(
        app.world().resource::<SelectedTargets>().0,
        SelectionReadModel::from_legacy_target(Some(anchor(FIRST)))
    );
}

#[test]
fn snapshot_reduces_complete_selection_for_reconnect() {
    let mut app = selection_test_app();
    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::ReplaceSelection {
            targets: vec![anchor(THIRD), anchor(FIRST), anchor(SECOND)],
            primary: Some(anchor(THIRD)),
        },
    );
    app.update();
    let _ = next_event(&mut app);

    app.world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::RequestSnapshot);
    app.update();
    let snapshot = next_event(&mut app);
    let ViewportEvent::Snapshot { state } = &snapshot.event else {
        panic!("reconnect must receive a snapshot");
    };
    assert_eq!(state.selection.targets.len(), 3);
    assert_eq!(state.selection.primary, Some(anchor(THIRD)));

    let mut reducer = ViewportReadModelState::default();
    reducer.apply(&snapshot);
    assert_eq!(
        reducer
            .snapshot()
            .expect("snapshot reducer state")
            .selection,
        state.selection
    );
}
