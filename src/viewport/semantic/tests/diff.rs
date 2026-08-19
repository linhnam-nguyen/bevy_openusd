use anyhow::Result;
use bevy::prelude::*;
use usd_bevy::LiveStage;
use usd_model::EntityKey;

use super::fixtures::response;
use super::super::{
    SemanticDiffState, SemanticResponse, SemanticSyncState, SemanticWorkingStore,
    synchronize_live_stage,
};

#[test]
fn test_regression_diff_after_subtree_resync_reflects_affected_only() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let usda = String::from(
        r#"#usda 1.0
def Xform "World"
{
    def Xform "A"
    {
        def Xform "OldMesh" {}
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
    app.insert_resource(SemanticDiffState::default());
    app.world_mut().insert_non_send(live);
    app.add_systems(PostUpdate, synchronize_live_stage);

    // Frame 1: Initial load
    app.update();
    assert!(matches!(
        response(app.world().resource::<SemanticWorkingStore>()),
        SemanticResponse::SnapshotLoaded { .. }
    ));

    // Capture baseline
    assert!(
        app.world_mut()
            .resource_mut::<SemanticDiffState>()
            .capture_baseline()
    );

    // Mutate /World/A: add /World/A/NewPrim
    {
        let live_ref = app.world().get_non_send::<LiveStage>().unwrap();
        let new_prim = live_ref.stage.define_prim("/World/A/NewPrim").unwrap();
        new_prim.set_type_name("Xform").unwrap();
        let _ = live_ref.drain_change_batch();
        live_ref.load_payload("/World/A");
    }

    // Frame 2: Subtree delta
    app.update();
    assert!(matches!(
        response(app.world().resource::<SemanticWorkingStore>()),
        SemanticResponse::DeltaApplied { .. }
    ));

    // Verify diff reflects A change while B is unaffected
    let diff_state = app.world().resource::<SemanticDiffState>();
    let diff = diff_state.stage_diff().expect("diff computed");
    let a_new_key = EntityKey::from("/World/A/NewPrim");
    let b_key = EntityKey::from("/World/B/MeshB");
    assert!(
        diff.entities.contains_key(&a_new_key),
        "diff contains /World/A/NewPrim"
    );
    let a_diff = diff.entities.get(&a_new_key).unwrap();
    assert_eq!(a_diff.presence, usd_model::PresenceState::Added);

    let b_diff = diff
        .entities
        .get(&b_key)
        .expect("/World/B/MeshB present in diff");
    assert_eq!(b_diff.presence, usd_model::PresenceState::Existing);
    assert!(
        !b_diff.is_changed(),
        "unaffected /World/B/MeshB must have is_changed == false"
    );
    assert_eq!(
        b_diff.flags,
        usd_model::ChangeFlags::empty(),
        "unaffected /World/B/MeshB has empty change flags"
    );

    Ok(())
}
