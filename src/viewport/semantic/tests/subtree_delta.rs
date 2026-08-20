use anyhow::Result;
use bevy::prelude::*;
use openusd::usd::Stage;
use usd_bevy::{LiveRevision, LiveStage, StageChange, StageChangeBatch};
use usd_model::{EntityKey, SnapshotSource};
use usd_semantic::{SemanticConfig, SemanticExtractor};

use super::super::{
    SemanticQuery, SemanticResponse, SemanticSyncState, SemanticWorkingStore,
    resync_subtree_update, synchronize_live_stage,
};
use super::fixtures::response;

#[test]
fn test_regression_a_resync_subtree_34_prims_delta_applied() -> Result<()> {
    let mut usda = String::from("#usda 1.0\n\ndef Xform \"World\"\n{\n");
    for group in ["A", "B", "C"] {
        usda.push_str(&format!("    def Xform \"{group}\"\n    {{\n"));
        for i in 0..10 {
            usda.push_str(&format!(
                "        def Xform \"{group}{i}\"\n        {{\n        }}\n"
            ));
        }
        usda.push_str("    }\n");
    }
    usda.push_str("}\n");

    let stage = usd_bevy::UsdSnippet::new(&usda)
        .open_stage()
        .expect("synthetic wide stage opens");
    let live = LiveStage::new(stage);

    let mut app = App::new();
    app.add_plugins(usd_bevy::LiveStagePlugin);
    app.insert_resource(SemanticWorkingStore::default());
    app.insert_resource(SemanticSyncState::default());
    app.world_mut().insert_non_send(live);
    app.add_systems(PostUpdate, synchronize_live_stage);

    app.update();
    let resp = response(app.world().resource::<SemanticWorkingStore>());
    let initial_count = match resp {
        SemanticResponse::SnapshotLoaded { entity_count, .. } => entity_count,
        other => panic!("expected initial SnapshotLoaded, got {other:?}"),
    };
    assert_eq!(initial_count, 34);

    // Enqueue resync on /World/B (11 prims affected: /World/B + 10 children)
    app.world()
        .get_non_send::<LiveStage>()
        .unwrap()
        .load_payload("/World/B");

    app.update();

    let resp = response(app.world().resource::<SemanticWorkingStore>());
    match resp {
        SemanticResponse::DeltaApplied {
            upserted, removed, ..
        } => {
            assert_eq!(upserted, 11);
            assert_eq!(removed, 0);
        }
        other => panic!("expected DeltaApplied for subtree resync, got {other:?}"),
    }

    // Confirm total rows in Turso is still 34
    let store = app.world().resource::<SemanticWorkingStore>();
    assert!(store.submit_query("verify-all-34", SemanticQuery::default()));
    let SemanticResponse::QueryResult { result, .. } = response(store) else {
        panic!("expected query result")
    };
    assert_eq!(result.total, 34);
    Ok(())
}

#[test]
fn test_regression_b_remove_and_add_under_resync_root() -> Result<()> {
    let stage = Stage::builder()
        .in_memory("semantic-b-remove-add.usda")
        .expect("in-memory stage");

    stage.define_prim("/World").unwrap();
    stage.define_prim("/World/A").unwrap();
    stage.define_prim("/World/A/Child1").unwrap();
    stage.define_prim("/World/A/Child2").unwrap();
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
            entity_count: 5,
            ..
        }
    ));

    // Remove Child2 and define Child3 under /World/A
    let live = app.world().get_non_send::<LiveStage>().unwrap();
    live.stage.remove_prim("/World/A/Child2").unwrap();
    live.stage.define_prim("/World/A/Child3").unwrap();
    let _ = live.drain_change_batch();
    live.load_payload("/World/A");

    app.update();

    let resp = response(app.world().resource::<SemanticWorkingStore>());
    match resp {
        SemanticResponse::DeltaApplied {
            upserted, removed, ..
        } => {
            // /World/A, /World/A/Child1, /World/A/Child3 = 3 upserts; /World/A/Child2 = 1 removal
            assert_eq!(upserted, 3);
            assert_eq!(removed, 1);
        }
        other => panic!("expected DeltaApplied, got {other:?}"),
    }

    // Query Turso: total 5 rows (/World, /World/A, /World/A/Child1, /World/A/Child3, /World/B)
    let store = app.world().resource::<SemanticWorkingStore>();
    assert!(store.submit_query("verify-count-5", SemanticQuery::default()));
    let SemanticResponse::QueryResult { result, .. } = response(store) else {
        panic!("expected query result")
    };
    assert_eq!(result.total, 5);
    Ok(())
}

#[test]
fn test_regression_c_resync_subtree_with_unshaded_changed_info() -> Result<()> {
    let stage = Stage::builder()
        .in_memory("semantic-c-mixed.usda")
        .expect("in-memory stage");

    stage.define_prim("/World").unwrap();
    stage.define_prim("/World/A").unwrap();
    stage.define_prim("/World/A/Child").unwrap();
    stage.define_prim("/World/B").unwrap();
    stage.define_prim("/World/B/Child").unwrap();

    let extractor = SemanticExtractor::new(SemanticConfig::default());
    let source = SnapshotSource::Working {
        session: "test-c".to_owned(),
        live_revision: 1,
    };
    let mut initial_snapshot = extractor.extract(&stage, source.clone())?;
    assert_eq!(initial_snapshot.entities.len(), 5);

    // Mutate initial_snapshot so /World/B/Child has a genuinely different old key
    let old_key = EntityKey::new("revit:old-unique-id-999");
    let original_key = initial_snapshot
        .entities
        .iter()
        .find(|(_, e)| e.prim_path == "/World/B/Child")
        .map(|(k, _)| k.clone())
        .unwrap();
    let mut b_child_entity = initial_snapshot.entities.remove(&original_key).unwrap();
    b_child_entity.key = old_key.clone();
    initial_snapshot
        .entities
        .insert(old_key.clone(), b_child_entity);

    // Mutate /World/A/Child and /World/B/Child on stage
    let live = LiveStage::new(stage);
    let _ = live.drain_change_batch();

    let mut batch = StageChangeBatch {
        revision: LiveRevision(2),
        changes: Vec::new(),
    };
    batch.changes.push(StageChange {
        changed_info: vec!["/World/B/Child.xformOp:translate".to_owned()],
        resynced: vec!["/World/A".to_owned()],
    });

    let delta = resync_subtree_update(&live.stage, &extractor, initial_snapshot, &batch, source)?;

    // Newly extracted key for /World/B/Child
    let new_key = EntityKey::from("/World/B/Child");
    assert_ne!(old_key, new_key);

    // delta.upserts: exactly one /World/B/Child entity with key == new_key
    let b_child_upserts: Vec<_> = delta
        .request
        .upserts
        .iter()
        .filter(|e| e.prim_path == "/World/B/Child")
        .collect();
    assert_eq!(b_child_upserts.len(), 1);
    assert_eq!(b_child_upserts[0].key, new_key);

    // Total upserts = 2 (/World/A + /World/A/Child) + 1 (/World/B/Child) = 3
    assert_eq!(delta.request.upserts.len(), 3);
    assert_eq!(delta.request.removed_paths.len(), 0);

    // delta.snapshot: does NOT contain old_key, DOES contain new_key
    assert!(!delta.snapshot.entities.contains_key(&old_key));
    assert!(delta.snapshot.entities.contains_key(&new_key));

    // delta.snapshot has only one entity whose prim_path == /World/B/Child
    let b_child_in_snapshot: Vec<_> = delta
        .snapshot
        .entities
        .values()
        .filter(|e| e.prim_path == "/World/B/Child")
        .collect();
    assert_eq!(b_child_in_snapshot.len(), 1);
    assert_eq!(delta.snapshot.entities.len(), 5);
    Ok(())
}

#[test]
fn test_regression_e_unaffected_entity_and_hash_byte_identical() -> Result<()> {
    let stage = Stage::builder()
        .in_memory("semantic-e-unchanged.usda")
        .expect("in-memory stage");

    stage.define_prim("/World").unwrap();
    stage.define_prim("/World/A").unwrap();
    stage.define_prim("/World/A/Child").unwrap();
    stage.define_prim("/World/B").unwrap();
    stage.define_prim("/World/B/Child").unwrap();

    let extractor = SemanticExtractor::new(SemanticConfig::default());
    let source = SnapshotSource::Working {
        session: "test-e".to_owned(),
        live_revision: 1,
    };
    let initial_snapshot = extractor.extract(&stage, source.clone())?;

    let before_b = initial_snapshot
        .entities
        .values()
        .find(|e| e.prim_path == "/World/B")
        .cloned()
        .unwrap();
    let before_b_child = initial_snapshot
        .entities
        .values()
        .find(|e| e.prim_path == "/World/B/Child")
        .cloned()
        .unwrap();

    // Mutate /World/A only on stage
    let live = LiveStage::new(stage);
    let _ = live.drain_change_batch();

    let mut batch = StageChangeBatch {
        revision: LiveRevision(2),
        changes: Vec::new(),
    };
    batch.changes.push(StageChange {
        changed_info: Vec::new(),
        resynced: vec!["/World/A".to_owned()],
    });

    let delta = resync_subtree_update(&live.stage, &extractor, initial_snapshot, &batch, source)?;

    let after_b = delta
        .snapshot
        .entities
        .values()
        .find(|e| e.prim_path == "/World/B")
        .cloned()
        .unwrap();
    let after_b_child = delta
        .snapshot
        .entities
        .values()
        .find(|e| e.prim_path == "/World/B/Child")
        .cloned()
        .unwrap();

    // Byte-for-byte identical verification
    assert_eq!(before_b, after_b);
    assert_eq!(before_b.full_hash, after_b.full_hash);
    assert_eq!(before_b_child, after_b_child);
    assert_eq!(before_b_child.full_hash, after_b_child.full_hash);
    Ok(())
}
