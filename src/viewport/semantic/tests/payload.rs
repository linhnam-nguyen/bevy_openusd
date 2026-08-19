use anyhow::Result;
use bevy::prelude::*;
use usd_bevy::LiveStage;
use usd_model::EntityKey;

use super::fixtures::response;
use super::super::{SemanticResponse, SemanticSyncState, SemanticWorkingStore, synchronize_live_stage};

#[test]
fn test_regression_payload_load_and_unload_lifecycle() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let a_path = temp_dir.path().join("a.usda");
    let main_path = temp_dir.path().join("main.usda");

    std::fs::write(
        &a_path,
        r#"#usda 1.0
(
    defaultPrim = "A"
)
def "A"
{
    def Mesh "PayloadChild"
    {
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
    }
}
"#,
    )?;

    std::fs::write(
        &main_path,
        r#"#usda 1.0
def Xform "World"
{
    def Xform "A" (
        payload = @./a.usda@
    )
    {
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
}
"#,
    )?;

    let stage = openusd::usd::Stage::builder()
        .open(main_path.to_str().unwrap())
        .expect("stage opens with payload");
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

    // Frame 1: Initial load (payload composed)
    app.update();
    assert!(matches!(
        response(app.world().resource::<SemanticWorkingStore>()),
        SemanticResponse::SnapshotLoaded { .. }
    ));

    let prim_entities_1 = app.world().resource::<usd_bevy::PrimEntities>();
    assert!(
        prim_entities_1.entity("/World/A/PayloadChild").is_some(),
        "PayloadChild composed initially in PrimEntities"
    );
    let b_bevy_before = prim_entities_1.entity("/World/B/MeshB").unwrap();

    let sync_state_1 = app.world().resource::<SemanticSyncState>();
    let snap_1 = sync_state_1.snapshot.as_ref().unwrap();
    assert!(
        snap_1
            .entities
            .contains_key(&EntityKey::from("/World/A/PayloadChild")),
        "PayloadChild composed initially in semantic entities"
    );
    let b_sem_before = snap_1
        .entities
        .get(&EntityKey::from("/World/B/MeshB"))
        .cloned()
        .unwrap();

    // Unload payload under /World/A
    {
        let live_ref = app.world().get_non_send::<LiveStage>().unwrap();
        live_ref.unload_payload("/World/A");
    }
    app.update();
    assert!(matches!(
        response(app.world().resource::<SemanticWorkingStore>()),
        SemanticResponse::DeltaApplied { .. }
    ));

    // After unload: PayloadChild is absent from Bevy ECS & semantic snapshot
    let prim_entities_2 = app.world().resource::<usd_bevy::PrimEntities>();
    assert!(
        prim_entities_2.entity("/World/A/PayloadChild").is_none(),
        "PayloadChild despawned after payload unload"
    );
    let sync_state_2 = app.world().resource::<SemanticSyncState>();
    let snap_2 = sync_state_2.snapshot.as_ref().unwrap();
    assert!(
        !snap_2
            .entities
            .contains_key(&EntityKey::from("/World/A/PayloadChild")),
        "PayloadChild absent from semantic snapshot after unload"
    );
    assert_eq!(
        prim_entities_2.entity("/World/B/MeshB").unwrap(),
        b_bevy_before,
        "sibling /World/B Bevy Entity invariant after unload"
    );

    // Load payload under /World/A
    {
        let live_ref = app.world().get_non_send::<LiveStage>().unwrap();
        live_ref.load_payload("/World/A");
    }
    app.update();
    assert!(matches!(
        response(app.world().resource::<SemanticWorkingStore>()),
        SemanticResponse::DeltaApplied { .. }
    ));

    // After load: PayloadChild is restored in Bevy ECS & semantic snapshot
    let prim_entities_3 = app.world().resource::<usd_bevy::PrimEntities>();
    assert!(
        prim_entities_3.entity("/World/A/PayloadChild").is_some(),
        "PayloadChild restored after payload load"
    );
    let sync_state_3 = app.world().resource::<SemanticSyncState>();
    let snap_3 = sync_state_3.snapshot.as_ref().unwrap();
    assert!(
        snap_3
            .entities
            .contains_key(&EntityKey::from("/World/A/PayloadChild")),
        "PayloadChild restored in semantic snapshot after load"
    );
    assert_eq!(
        prim_entities_3.entity("/World/B/MeshB").unwrap(),
        b_bevy_before,
        "sibling /World/B Bevy Entity invariant after load"
    );
    let b_sem_after = snap_3
        .entities
        .get(&EntityKey::from("/World/B/MeshB"))
        .cloned()
        .unwrap();
    assert_eq!(
        b_sem_before, b_sem_after,
        "sibling /World/B semantic content invariant across payload lifecycle"
    );

    Ok(())
}
