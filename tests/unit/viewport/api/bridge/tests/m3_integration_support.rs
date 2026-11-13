use std::thread::sleep;
use std::time::Duration;

use anyhow::{Result, bail};
use bevy::prelude::*;
use openusd::sdf::Value;
use openusd::usd::Stage;
use usd_model::{CanonicalValue, SemanticSnapshot, SnapshotSource};
use viewport_protocol::{SceneAnchor, SelectionReadModel, ViewportEventEnvelope};

use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::scene::SelectedTargets;
use crate::viewport::semantic::{SemanticDiffState, SemanticSyncState};

pub(super) fn stage_with_widths() -> Stage {
    let stage = Stage::builder()
        .in_memory("m3_integration.usda")
        .expect("stage opens");
    stage
        .define_prim("/World")
        .expect("world defines")
        .set_type_name("Xform")
        .expect("world type authors");
    for (path, width) in [("/World/A", 1.0), ("/World/B", 2.0)] {
        let prim = stage
            .define_prim(path)
            .expect("target defines")
            .set_type_name("Xform")
            .expect("target type authors");
        prim.create_attribute("BIM:Instance:ElementId", "string")
            .expect("element id attribute creates")
            .set_custom(true)
            .expect("element id custom flag authors")
            .set(Value::String(path.rsplit('/').next().unwrap().to_owned()))
            .expect("element id authors");
        prim.create_attribute("Width", "double")
            .expect("width attribute creates")
            .set_custom(true)
            .expect("width custom flag authors")
            .set(Value::Double(width))
            .expect("width authors");
    }
    stage
}

pub(super) fn anchor(path: &str) -> SceneAnchor {
    SceneAnchor::active_session(path)
}

pub(super) fn mutation(
    path: &str,
    expected: f64,
    next: f64,
) -> viewport_protocol::BimPropertyMutation {
    viewport_protocol::BimPropertyMutation {
        target: anchor(path),
        property: "Width".to_owned(),
        value: serde_json::json!(next),
        input_unit: None,
        expected_old_value: CanonicalValue::Real(expected),
    }
}

pub(super) fn select_targets(app: &mut App, paths: &[&str]) -> u64 {
    let targets = paths.iter().map(|path| anchor(path)).collect::<Vec<_>>();
    let mut selection = SelectedTargets::default();
    selection
        .replace(SelectionReadModel {
            primary: targets.first().cloned(),
            targets,
        })
        .expect("selection is valid");
    let revision = selection.revision();
    app.world_mut().insert_resource(selection);
    revision
}

pub(super) fn read_width(app: &App, path: &str) -> f64 {
    let live = app
        .world()
        .get_non_send::<usd_bevy::LiveStage>()
        .expect("live stage");
    match live
        .stage
        .prim(openusd::sdf::path(path).expect("path parses"))
        .attribute("Width")
        .get::<Value>()
        .expect("width reads")
    {
        Some(Value::Double(value)) => value,
        other => panic!("expected double Width, got {other:?}"),
    }
}

pub(super) fn semantic_width(app: &App, path: &str) -> CanonicalValue {
    app.world()
        .resource::<SemanticSyncState>()
        .snapshot()
        .expect("semantic snapshot")
        .entities
        .values()
        .find(|entity| entity.prim_path == path)
        .and_then(|entity| {
            entity
                .properties
                .iter()
                .find(|property| property.name == "Width")
        })
        .map(|property| property.value.clone())
        .expect("semantic Width property")
}

fn snapshot_revision(snapshot: &SemanticSnapshot) -> Option<u64> {
    match &snapshot.source {
        SnapshotSource::Working { live_revision, .. } => Some(*live_revision),
        SnapshotSource::GitCommit { .. } => None,
    }
}

pub(super) fn wait_for_initial_semantics(app: &mut App) -> Result<SemanticSnapshot> {
    for _ in 0..200 {
        app.update();
        if let Some(snapshot) = app
            .world()
            .resource::<SemanticSyncState>()
            .snapshot()
            .cloned()
        {
            return Ok(snapshot);
        }
        sleep(Duration::from_millis(5));
    }
    bail!("initial semantic snapshot did not load")
}

pub(super) fn wait_for_event(app: &mut App, request_id: &str) -> Result<ViewportEventEnvelope> {
    for _ in 0..200 {
        app.update();
        while let Some(event) = app.world_mut().resource_mut::<ViewportEventOutbox>().pop() {
            if event.request_id.as_deref() == Some(request_id) {
                return Ok(event);
            }
        }
        sleep(Duration::from_millis(5));
    }
    bail!("request event did not arrive: {request_id}")
}

pub(super) fn wait_for_change(
    app: &mut App,
    request_id: &str,
    previous_revision: u64,
) -> Result<(ViewportEventEnvelope, usd_bevy::StageChangeBatch)> {
    let mut matched_event = None;
    for _ in 0..200 {
        app.update();
        while let Some(event) = app.world_mut().resource_mut::<ViewportEventOutbox>().pop() {
            if event.request_id.as_deref() == Some(request_id) {
                matched_event = Some(event);
                break;
            }
        }
        let live_revision = app
            .world()
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("live stage")
            .current_revision()
            .0;
        let semantic_revision = app
            .world()
            .resource::<SemanticSyncState>()
            .snapshot()
            .and_then(snapshot_revision);
        if matched_event.is_some()
            && live_revision > previous_revision
            && semantic_revision == Some(live_revision)
        {
            let event = matched_event.take().expect("matched event is present");
            let batch = app
                .world()
                .resource::<usd_bevy::PendingStageChanges>()
                .batch()
                .cloned()
                .expect("semantic update retains the StageChangeBatch");
            assert_eq!(batch.revision.0, live_revision);
            return Ok((event, batch));
        }
        if matched_event.is_none() {
            sleep(Duration::from_millis(5));
        }
    }
    bail!("integrated semantic change did not converge: {request_id}")
}

pub(super) fn assert_modified_diff(app: &App, path: &str) {
    let diff = app
        .world()
        .resource::<SemanticDiffState>()
        .bim_property_diff(&[anchor(path)])
        .expect("Git-baseline BIM diff");
    assert_eq!(
        diff.status,
        viewport_protocol::BimPropertyDiffStatus::Modified
    );
    assert!(diff.properties.iter().any(|property| {
        property.key == "Width"
            && property.status == viewport_protocol::BimPropertyDiffStatus::Modified
    }));
}
