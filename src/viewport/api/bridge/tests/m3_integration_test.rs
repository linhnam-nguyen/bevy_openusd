use anyhow::Result;
use bevy::prelude::*;
use openusd::sdf::Value;
use openusd::usd::Stage;
use usd_model::{CanonicalValue, SnapshotSource};
use viewport_protocol::{BimPropertyEditStatus, EditorOperation, ViewportCommand, ViewportEvent};

use crate::viewport::api::ViewportCommandInbox;
use crate::viewport::api::bridge::state::EditorHistories;
use crate::viewport::semantic::SemanticDiffState;
use crate::viewport::session::StageHandle;

use super::m3_integration_support::*;
use super::support::runtime_semantic_test_app;

#[test]
fn m3_edit_semantic_diff_undo_redo_save_replace_and_batch_converge() -> Result<()> {
    let project_root = tempfile::tempdir()?;
    let save_path = project_root.path().join("m3-saved.usda");
    let mut app = runtime_semantic_test_app(project_root.path().to_path_buf());
    configure_bim_runtime_semantics(&mut app);
    app.world_mut()
        .insert_non_send(usd_bevy::LiveStage::new(stage_with_widths()));

    let initial = wait_for_initial_semantics(&mut app)?;
    let mut baseline = initial.clone();
    baseline.source = SnapshotSource::GitCommit {
        oid: "m3-integration-baseline".to_owned(),
    };
    assert!(
        app.world_mut()
            .resource_mut::<SemanticDiffState>()
            .set_git_baseline(baseline)
    );

    select_targets(&mut app, &["/World/A"]);
    let previous_revision = app
        .world()
        .get_non_send::<usd_bevy::LiveStage>()
        .expect("live stage")
        .current_revision()
        .0;
    let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::EditBimProperty {
            mutation: mutation("/World/A", 1.0, 10.0),
        },
    );
    let (event, batch) = wait_for_change(&mut app, &request_id, previous_revision)?;
    assert!(matches!(
        event.event,
        ViewportEvent::BimPropertyEditCompleted {
            outcome,
            ..
        } if outcome.status == BimPropertyEditStatus::Applied
    ));
    assert!(
        batch
            .changes
            .iter()
            .flat_map(|change| &change.changed_info)
            .any(|path| path == "/World/A.Width")
    );
    assert_eq!(read_width(&app, "/World/A"), 10.0);
    assert_eq!(semantic_width(&app, "/World/A"), CanonicalValue::Real(10.0));
    assert_modified_diff(&app, "/World/A");

    let previous_revision = app
        .world()
        .get_non_send::<usd_bevy::LiveStage>()
        .expect("live stage")
        .current_revision()
        .0;
    let undo_request = app
        .world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::UndoEditor);
    let (event, _) = wait_for_change(&mut app, &undo_request, previous_revision)?;
    assert!(matches!(
        event.event,
        ViewportEvent::EditorCommandCompleted {
            operation: EditorOperation::Undo,
            ..
        }
    ));
    assert_eq!(read_width(&app, "/World/A"), 1.0);
    assert_eq!(semantic_width(&app, "/World/A"), CanonicalValue::Real(1.0));
    let diff = app
        .world()
        .resource::<SemanticDiffState>()
        .bim_property_diff(&[anchor("/World/A")])
        .expect("Git-baseline BIM diff after undo");
    assert_eq!(
        diff.status,
        viewport_protocol::BimPropertyDiffStatus::Unchanged
    );

    let previous_revision = app
        .world()
        .get_non_send::<usd_bevy::LiveStage>()
        .expect("live stage")
        .current_revision()
        .0;
    let redo_request = app
        .world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::RedoEditor);
    let (event, _) = wait_for_change(&mut app, &redo_request, previous_revision)?;
    assert!(matches!(
        event.event,
        ViewportEvent::EditorCommandCompleted {
            operation: EditorOperation::Redo,
            ..
        }
    ));
    assert_eq!(read_width(&app, "/World/A"), 10.0);
    assert_eq!(semantic_width(&app, "/World/A"), CanonicalValue::Real(10.0));
    assert_modified_diff(&app, "/World/A");

    app.world_mut().insert_resource(StageHandle {
        path: save_path.clone(),
        error: None,
    });
    let save_request = app
        .world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::SaveStage);
    let save_event = wait_for_event(&mut app, &save_request)?;
    assert!(matches!(
        save_event.event,
        ViewportEvent::EditorCommandCompleted {
            operation: EditorOperation::SaveStage,
            changed_paths,
            ..
        } if changed_paths.is_empty()
    ));
    let reopened = Stage::open(save_path.to_str().expect("save path is UTF-8"))?;
    assert!(matches!(
        reopened
            .prim(openusd::sdf::path("/World/A")?)
            .attribute("Width")
            .get::<Value>()?,
        Some(Value::Double(value)) if value == 10.0
    ));
    assert!(app.world().resource::<SemanticDiffState>().has_baseline());
    assert_modified_diff(&app, "/World/A");

    let previous_revision = app
        .world()
        .get_non_send::<usd_bevy::LiveStage>()
        .expect("live stage")
        .current_revision()
        .0;
    let replacement_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::EditBimProperty {
            mutation: mutation("/World/A", 10.0, 12.0),
        },
    );
    let (event, _) = wait_for_change(&mut app, &replacement_request, previous_revision)?;
    assert!(matches!(
        event.event,
        ViewportEvent::BimPropertyEditCompleted {
            outcome,
            ..
        } if outcome.status == BimPropertyEditStatus::Applied
    ));
    assert_eq!(read_width(&app, "/World/A"), 12.0);
    assert_modified_diff(&app, "/World/A");

    let selection_revision = select_targets(&mut app, &["/World/A", "/World/B"]);
    let stale_revision = selection_revision.saturating_sub(1);
    let before_rejected_revision = app
        .world()
        .get_non_send::<usd_bevy::LiveStage>()
        .expect("live stage")
        .current_revision()
        .0;
    let before_rejected_history = app.world().resource::<EditorHistories>().undo_domains.len();
    let rejected_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::EditBimProperties {
            selection_revision: stale_revision,
            mutations: vec![
                mutation("/World/A", 12.0, 20.0),
                mutation("/World/B", 2.0, 30.0),
            ],
        },
    );
    let rejected_event = wait_for_event(&mut app, &rejected_request)?;
    assert!(matches!(
        rejected_event.event,
        ViewportEvent::BimPropertyBatchEditCompleted {
            applied: false,
            outcomes,
            ..
        } if outcomes.iter().all(|outcome| outcome.status == BimPropertyEditStatus::Rejected)
    ));
    assert_eq!(
        app.world()
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("live stage")
            .current_revision()
            .0,
        before_rejected_revision
    );
    assert_eq!(read_width(&app, "/World/A"), 12.0);
    assert_eq!(read_width(&app, "/World/B"), 2.0);
    assert_eq!(
        app.world().resource::<EditorHistories>().undo_domains.len(),
        before_rejected_history
    );

    let previous_revision = app
        .world()
        .get_non_send::<usd_bevy::LiveStage>()
        .expect("live stage")
        .current_revision()
        .0;
    let batch_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::EditBimProperties {
            selection_revision,
            mutations: vec![
                mutation("/World/A", 12.0, 20.0),
                mutation("/World/B", 2.0, 30.0),
            ],
        },
    );
    let (event, batch) = wait_for_change(&mut app, &batch_request, previous_revision)?;
    assert!(matches!(
        event.event,
        ViewportEvent::BimPropertyBatchEditCompleted {
            applied: true,
            outcomes,
            ..
        } if outcomes.len() == 2
            && outcomes.iter().all(|outcome| outcome.status == BimPropertyEditStatus::Applied)
    ));
    assert!(
        batch
            .changes
            .iter()
            .flat_map(|change| &change.changed_info)
            .any(|path| path == "/World/A.Width")
    );
    assert!(
        batch
            .changes
            .iter()
            .flat_map(|change| &change.changed_info)
            .any(|path| path == "/World/B.Width")
    );
    assert_eq!(read_width(&app, "/World/A"), 20.0);
    assert_eq!(read_width(&app, "/World/B"), 30.0);

    let previous_revision = app
        .world()
        .get_non_send::<usd_bevy::LiveStage>()
        .expect("live stage")
        .current_revision()
        .0;
    let batch_undo_request = app
        .world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::UndoEditor);
    let (event, _) = wait_for_change(&mut app, &batch_undo_request, previous_revision)?;
    assert!(matches!(
        event.event,
        ViewportEvent::EditorCommandCompleted {
            operation: EditorOperation::Undo,
            ..
        }
    ));
    assert_eq!(read_width(&app, "/World/A"), 12.0);
    assert_eq!(read_width(&app, "/World/B"), 2.0);
    assert_modified_diff(&app, "/World/A");

    Ok(())
}
