use anyhow::Result;
use bevy::prelude::*;
use openusd::usd::Stage;
use usd_bevy::{LiveRevision, LiveStage, StageChange, StageChangeBatch};
use usd_model::SnapshotSource;
use usd_semantic::{SemanticConfig, SemanticExtractor};

use super::fixtures::response;
use super::super::{
    SemanticResponse, SemanticSyncState, SemanticWorkingStore, SubtreeUpdateError,
    resync_subtree_update, synchronize_live_stage,
};

#[test]
fn test_regression_d_collision_triggers_full_snapshot_fallback() -> Result<()> {
    let stage = Stage::builder()
        .in_memory("semantic-d-collision.usda")
        .expect("in-memory stage");

    stage.define_prim("/World").unwrap();
    stage.define_prim("/World/A").unwrap();
    stage.define_prim("/World/B").unwrap();

    let live = LiveStage::new(stage);

    let mut app = App::new();
    app.add_plugins(usd_bevy::LiveStagePlugin);
    app.insert_resource(SemanticWorkingStore::default());
    app.insert_resource(SemanticSyncState::default());
    app.world_mut().insert_non_send(live);
    app.add_systems(PostUpdate, synchronize_live_stage);

    app.update();
    let resp = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(
        resp,
        SemanticResponse::SnapshotLoaded {
            entity_count: 3,
            ..
        }
    ));

    // Artificially modify SemanticSyncState so /World/B has the same key as /World/A
    {
        let mut sync_state = app.world_mut().resource_mut::<SemanticSyncState>();
        let mut snapshot = sync_state.snapshot.take().unwrap();
        let a_key = snapshot
            .entities
            .iter()
            .find(|(_, e)| e.prim_path == "/World/A")
            .map(|(k, _)| k.clone())
            .unwrap();
        let b_key = snapshot
            .entities
            .iter()
            .find(|(_, e)| e.prim_path == "/World/B")
            .map(|(k, _)| k.clone())
            .unwrap();
        let mut b_entity = snapshot.entities.remove(&b_key).unwrap();
        b_entity.key = a_key.clone();
        snapshot.entities.insert(a_key, b_entity);
        sync_state.snapshot = Some(snapshot);
    }

    // Resync /World/A -> Extracted /World/A will collide with unaffected /World/B's artificial key
    // -> resync_subtree_update returns Err -> synchronize_live_stage catches Err and does full fallback
    let live = app.world().get_non_send::<LiveStage>().unwrap();
    live.load_payload("/World/A");

    app.update();

    let resp = response(app.world().resource::<SemanticWorkingStore>());
    match resp {
        SemanticResponse::SnapshotLoaded { entity_count, .. } => {
            // Successfully fell back to full snapshot rebuild
            assert_eq!(entity_count, 3);
        }
        other => panic!("expected full SnapshotLoaded fallback on collision, got {other:?}"),
    }
    Ok(())
}

#[test]
fn test_regression_d2_direct_current_current_collision_error() -> Result<()> {
    let usda = r#"#usda 1.0
def Xform "World"
{
    def Xform "A"
    {
        def Xform "Child1"
        {
            string revit:uniqueId = "dup-123"
        }
        def Xform "Child2"
        {
            string revit:uniqueId = "dup-123"
        }
    }
}
"#;
    let stage = usd_bevy::UsdSnippet::new(usda)
        .open_stage()
        .expect("stage opens");

    let mut config = SemanticConfig::default();
    config.identity.revit_unique_id_candidates = vec!["revit:uniqueId".to_string()];
    let extractor = SemanticExtractor::new(config);
    let source = SnapshotSource::Working {
        session: "test-d2".to_owned(),
        live_revision: 1,
    };

    let initial_snapshot = extractor.snapshot_from_entities(source.clone(), Default::default());

    let mut batch = StageChangeBatch {
        revision: LiveRevision(2),
        changes: Vec::new(),
    };
    batch.changes.push(StageChange {
        changed_info: Vec::new(),
        resynced: vec!["/World/A".to_owned()],
    });

    let result = resync_subtree_update(&stage, &extractor, initial_snapshot, &batch, source);
    assert!(matches!(
        result,
        Err(SubtreeUpdateError::EntityKeyCollision(_))
    ));
    Ok(())
}

#[test]
fn test_regression_f_root_resync_triggers_full_snapshot_load() -> Result<()> {
    let stage = Stage::builder()
        .in_memory("semantic-f-root.usda")
        .expect("in-memory stage");

    stage.define_prim("/World").unwrap();
    stage.define_prim("/World/A").unwrap();
    stage.define_prim("/World/B").unwrap();

    let live = LiveStage::new(stage);

    let mut app = App::new();
    app.add_plugins(usd_bevy::LiveStagePlugin);
    app.insert_resource(SemanticWorkingStore::default());
    app.insert_resource(SemanticSyncState::default());
    app.world_mut().insert_non_send(live);
    app.add_systems(PostUpdate, synchronize_live_stage);

    app.update();
    let resp = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(
        resp,
        SemanticResponse::SnapshotLoaded {
            entity_count: 3,
            ..
        }
    ));

    // Resync root "/"
    let live = app.world().get_non_send::<LiveStage>().unwrap();
    live.load_payload("/");

    app.update();

    let resp = response(app.world().resource::<SemanticWorkingStore>());
    match resp {
        SemanticResponse::SnapshotLoaded { entity_count, .. } => {
            assert_eq!(entity_count, 3);
        }
        other => panic!("expected SnapshotLoaded for root resync, got {other:?}"),
    }
    Ok(())
}

#[test]
fn test_fallback_unnormalizable_root_triggers_full_snapshot_replace() -> Result<()> {
    let usda = String::from(
        r#"#usda 1.0
def Xform "World"
{
    def Xform "A" {}
}
"#,
    );
    let stage = usd_bevy::UsdSnippet::new(&usda)
        .open_stage()
        .expect("stage opens");
    let live = LiveStage::new(stage);

    let mut app = App::new();
    app.add_plugins(usd_bevy::LiveStagePlugin);
    app.insert_resource(SemanticWorkingStore::default());
    app.insert_resource(SemanticSyncState::default());
    app.world_mut().insert_non_send(live);
    app.add_systems(PostUpdate, synchronize_live_stage);

    // Frame 1: Full replace load
    app.update();
    let resp = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp, SemanticResponse::SnapshotLoaded { .. }));

    // Actually enqueue unnormalizable invalid root into LiveStage
    app.world()
        .get_non_send::<LiveStage>()
        .unwrap()
        .enqueue_resync("/World/Invalid..Path///");

    // Frame 2: Updates and falls back to full SnapshotLoaded (strictly SnapshotLoaded, NOT DeltaApplied)
    app.update();
    let resp = response(app.world().resource::<SemanticWorkingStore>());
    assert!(
        matches!(resp, SemanticResponse::SnapshotLoaded { .. }),
        "expected full SnapshotLoaded fallback, got {resp:?}"
    );
    Ok(())
}
