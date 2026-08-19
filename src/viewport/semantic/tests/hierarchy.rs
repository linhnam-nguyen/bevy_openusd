use anyhow::Result;
use bevy::prelude::*;
use usd_bevy::LiveStage;
use usd_model::EntityKey;

use super::super::{
    SemanticResponse, SemanticSyncState, SemanticWorkingStore, synchronize_live_stage,
};
use super::fixtures::response;

#[test]
fn test_regression_multiple_disjoint_roots_reconcile_and_semantic_invariance() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let usda = String::from(
        r#"#usda 1.0
def Xform "World"
{
    def Xform "A"
    {
        def Mesh "MeshA"
        {
            point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
            int[] faceVertexCounts = [3]
            int[] faceVertexIndices = [0, 1, 2]
        }
    }
    def Xform "B"
    {
        def Mesh "MeshB"
        {
            point3f[] points = [(10, 0, 0), (11, 0, 0), (10, 1, 0)]
            int[] faceVertexCounts = [3]
            int[] faceVertexIndices = [0, 1, 2]
        }
    }
    def Xform "C"
    {
        def Mesh "MeshC"
        {
            point3f[] points = [(20, 0, 0), (21, 0, 0), (20, 1, 0)]
            int[] faceVertexCounts = [3]
            int[] faceVertexIndices = [0, 1, 2]
        }
    }
}
"#,
    );
    let stage = usd_bevy::UsdSnippet::new(&usda)
        .open_stage()
        .expect("stage opens");
    let live = LiveStage::new(stage);

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<bevy::image::Image>()
        .init_asset::<StandardMaterial>()
        .add_plugins(usd_bevy::UsdPlugin)
        .add_plugins(usd_bevy::LiveStagePlugin);
    app.insert_resource(crate::project::recovery::RecoverySettings {
        project_root: temp_dir.path().to_path_buf(),
    });
    app.insert_resource(crate::project::ghost_cache::HistoricalGeometryCache::default());
    app.insert_resource(SemanticWorkingStore::default());
    app.insert_resource(SemanticSyncState::default());
    app.world_mut().insert_non_send(live);
    app.add_systems(PostUpdate, synchronize_live_stage);

    app.update();
    let resp_1 = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp_1, SemanticResponse::SnapshotLoaded { .. }));

    // Capture /World/B state
    let prim_entities = app.world().resource::<usd_bevy::PrimEntities>();
    let b_bevy_before = prim_entities.entity("/World/B/MeshB").unwrap();
    let sync_state_1 = app.world().resource::<SemanticSyncState>();
    let b_sem_before = sync_state_1
        .snapshot
        .as_ref()
        .unwrap()
        .entities
        .get(&EntityKey::from("/World/B/MeshB"))
        .cloned()
        .unwrap();

    // Mutate /World/A and /World/C
    {
        let live_ref = app.world().get_non_send::<LiveStage>().unwrap();
        let new_a = live_ref.stage.define_prim("/World/A/ExtraA").unwrap();
        new_a.set_type_name("Xform").unwrap();
        let new_c = live_ref.stage.define_prim("/World/C/ExtraC").unwrap();
        new_c.set_type_name("Xform").unwrap();
        let _ = live_ref.drain_change_batch();
        live_ref.load_payload("/World/A");
        live_ref.load_payload("/World/C");
    }

    app.update();
    let resp_2 = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp_2, SemanticResponse::DeltaApplied { .. }));

    // Verify both /World/A and /World/C updated
    let sync_state_2 = app.world().resource::<SemanticSyncState>();
    let snap_2 = sync_state_2.snapshot.as_ref().unwrap();
    assert!(
        snap_2
            .entities
            .contains_key(&EntityKey::from("/World/A/ExtraA"))
    );
    assert!(
        snap_2
            .entities
            .contains_key(&EntityKey::from("/World/C/ExtraC"))
    );

    // Verify /World/B is completely unchanged
    let prim_entities_2 = app.world().resource::<usd_bevy::PrimEntities>();
    let b_bevy_after = prim_entities_2.entity("/World/B/MeshB").unwrap();
    assert_eq!(
        b_bevy_before, b_bevy_after,
        "Bevy entity ID for /World/B invariant across disjoint resync"
    );
    let b_sem_after = snap_2
        .entities
        .get(&EntityKey::from("/World/B/MeshB"))
        .cloned()
        .unwrap();
    assert_eq!(
        b_sem_before, b_sem_after,
        "Semantic entity for /World/B invariant across disjoint resync"
    );

    Ok(())
}

#[test]
fn test_regression_prim_rename_preserves_sibling_and_migrates_entity() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let usda = String::from(
        r#"#usda 1.0
def Xform "World"
{
    def Xform "A"
    {
        def Xform "OldName" {}
    }
    def Xform "B"
    {
        def Xform "MeshB" {}
    }
}
"#,
    );
    let stage = usd_bevy::UsdSnippet::new(&usda)
        .open_stage()
        .expect("stage opens");
    let live = LiveStage::new(stage);

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<bevy::image::Image>()
        .init_asset::<StandardMaterial>()
        .add_plugins(usd_bevy::UsdPlugin)
        .add_plugins(usd_bevy::LiveStagePlugin);
    app.insert_resource(crate::project::recovery::RecoverySettings {
        project_root: temp_dir.path().to_path_buf(),
    });
    app.insert_resource(crate::project::ghost_cache::HistoricalGeometryCache::default());
    app.insert_resource(SemanticWorkingStore::default());
    app.insert_resource(SemanticSyncState::default());
    app.world_mut().insert_non_send(live);
    app.add_systems(PostUpdate, synchronize_live_stage);

    app.update();
    let resp_1 = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp_1, SemanticResponse::SnapshotLoaded { .. }));

    let prim_entities = app.world().resource::<usd_bevy::PrimEntities>();
    let b_bevy_before = prim_entities.entity("/World/B/MeshB").unwrap();

    // Rename /World/A/OldName to /World/A/NewName
    {
        let live_ref = app.world().get_non_send::<LiveStage>().unwrap();
        live_ref.stage.remove_prim("/World/A/OldName").unwrap();
        let new_prim = live_ref.stage.define_prim("/World/A/NewName").unwrap();
        new_prim.set_type_name("Xform").unwrap();
        let _ = live_ref.drain_change_batch();
        live_ref.load_payload("/World/A");
    }

    app.update();
    let resp_2 = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp_2, SemanticResponse::DeltaApplied { .. }));

    let prim_entities_2 = app.world().resource::<usd_bevy::PrimEntities>();
    assert!(
        prim_entities_2.entity("/World/A/OldName").is_none(),
        "old path removed from PrimEntities"
    );
    assert!(
        prim_entities_2.entity("/World/A/NewName").is_some(),
        "new path present in PrimEntities"
    );
    assert_eq!(
        prim_entities_2.entity("/World/B/MeshB").unwrap(),
        b_bevy_before,
        "sibling /World/B stable"
    );

    let sync_state_2 = app.world().resource::<SemanticSyncState>();
    let snap_2 = sync_state_2.snapshot.as_ref().unwrap();
    assert!(
        !snap_2
            .entities
            .contains_key(&EntityKey::from("/World/A/OldName"))
    );
    assert!(
        snap_2
            .entities
            .contains_key(&EntityKey::from("/World/A/NewName"))
    );

    Ok(())
}

#[test]
fn test_regression_prim_reparent_updates_hierarchy_and_preserves_sibling() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let usda = String::from(
        r#"#usda 1.0
def Xform "World"
{
    def Xform "A"
    {
        def Xform "Child" {}
    }
    def Xform "B"
    {
        def Xform "MeshB" {}
    }
    def Xform "C" {}
}
"#,
    );
    let stage = usd_bevy::UsdSnippet::new(&usda)
        .open_stage()
        .expect("stage opens");
    let live = LiveStage::new(stage);

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<bevy::image::Image>()
        .init_asset::<StandardMaterial>()
        .add_plugins(usd_bevy::UsdPlugin)
        .add_plugins(usd_bevy::LiveStagePlugin);
    app.insert_resource(crate::project::recovery::RecoverySettings {
        project_root: temp_dir.path().to_path_buf(),
    });
    app.insert_resource(crate::project::ghost_cache::HistoricalGeometryCache::default());
    app.insert_resource(SemanticWorkingStore::default());
    app.insert_resource(SemanticSyncState::default());
    app.world_mut().insert_non_send(live);
    app.add_systems(PostUpdate, synchronize_live_stage);

    app.update();
    let resp_1 = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp_1, SemanticResponse::SnapshotLoaded { .. }));

    let prim_entities = app.world().resource::<usd_bevy::PrimEntities>();
    let b_bevy_before = prim_entities.entity("/World/B/MeshB").unwrap();

    // Reparent /World/A/Child to /World/C/Child
    {
        let live_ref = app.world().get_non_send::<LiveStage>().unwrap();
        live_ref.stage.remove_prim("/World/A/Child").unwrap();
        let new_child = live_ref.stage.define_prim("/World/C/Child").unwrap();
        new_child.set_type_name("Xform").unwrap();
        let _ = live_ref.drain_change_batch();
        live_ref.load_payload("/World/A");
        live_ref.load_payload("/World/C");
    }

    app.update();
    let resp_2 = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp_2, SemanticResponse::DeltaApplied { .. }));

    let prim_entities_2 = app.world().resource::<usd_bevy::PrimEntities>();
    assert!(prim_entities_2.entity("/World/A/Child").is_none());
    let child_entity = prim_entities_2
        .entity("/World/C/Child")
        .expect("reparented child present in PrimEntities");
    let c_entity = prim_entities_2
        .entity("/World/C")
        .expect("/World/C entity in PrimEntities");
    let a_entity = prim_entities_2
        .entity("/World/A")
        .expect("/World/A entity in PrimEntities");
    assert_eq!(
        prim_entities_2.entity("/World/B/MeshB").unwrap(),
        b_bevy_before
    );

    // Verify Bevy hierarchy parent relation is strictly updated to /World/C
    let child_of = app
        .world()
        .get::<bevy::prelude::ChildOf>(child_entity)
        .expect("reparented child has ChildOf component");
    assert_eq!(
        child_of.parent(),
        c_entity,
        "Child must be parented to /World/C entity in Bevy hierarchy"
    );
    assert_ne!(
        child_of.parent(),
        a_entity,
        "Child must NOT be parented to /World/A entity in Bevy hierarchy"
    );

    let sync_state_2 = app.world().resource::<SemanticSyncState>();
    let snap_2 = sync_state_2.snapshot.as_ref().unwrap();
    assert!(
        !snap_2
            .entities
            .contains_key(&EntityKey::from("/World/A/Child"))
    );
    assert!(
        snap_2
            .entities
            .contains_key(&EntityKey::from("/World/C/Child"))
    );

    Ok(())
}
