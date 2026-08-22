use anyhow::Result;
use bevy::prelude::*;
use usd_bevy::{LiveStage, ProjectionReadiness};

use super::super::{
    RenderServerInterface, RuntimeDeliveryRuntime, SemanticSyncState, SemanticWorkingStore,
    drain_runtime_delivery_results, flush_pending_runtime_delivery, synchronize_live_stage,
};
use super::fixtures::response;
use super::runtime_delivery_support::wait_for_manifest;

#[test]
fn runtime_delivery_waits_for_projection_ready() -> Result<()> {
    let project = tempfile::tempdir()?;
    let stage = usd_bevy::UsdSnippet::new(
        r#"#usda 1.0
def Xform "World"
{
    def Xform "A" {}
    def Xform "B" {}
    def Xform "C" {}
}
"#,
    )
    .open_stage()?;
    let live = LiveStage::new(stage);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<bevy::image::Image>()
        .init_asset::<StandardMaterial>()
        .add_plugins(usd_bevy::UsdPlugin)
        .add_plugins(usd_bevy::LiveStagePlugin);
    app.insert_resource(usd_bevy::ProjectionBudget::work_items(1));
    app.insert_resource(crate::project::recovery::RecoverySettings {
        project_root: project.path().to_path_buf(),
    });
    app.insert_resource(crate::project::ghost_cache::HistoricalGeometryCache::default());
    app.insert_resource(SemanticWorkingStore::default());
    app.insert_resource(SemanticSyncState::default());
    app.insert_resource(RuntimeDeliveryRuntime::default());
    let interface = RenderServerInterface::default();
    app.insert_resource(interface.clone());
    app.world_mut().insert_non_send(live);
    app.add_systems(
        PostUpdate,
        (
            drain_runtime_delivery_results,
            synchronize_live_stage,
            flush_pending_runtime_delivery,
        )
            .chain(),
    );

    app.update();
    let _ = response(app.world().resource::<SemanticWorkingStore>());
    let shared_interface = interface.shared();
    assert_ne!(
        app.world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness(),
        ProjectionReadiness::Ready
    );
    assert!(shared_interface.runtime_manifest().is_none());

    let manifest = wait_for_manifest(&mut app, &shared_interface, None);
    assert_eq!(
        app.world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness(),
        ProjectionReadiness::Ready
    );
    assert!(!manifest.meshes.is_empty() || manifest.hierarchy.blob_id != "");
    Ok(())
}
