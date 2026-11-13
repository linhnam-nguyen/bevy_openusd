use anyhow::Result;
use bevy::prelude::*;
use usd_bevy::LiveStage;

use super::super::{
    RenderServerInterface, SemanticResponse, SemanticSyncState, SemanticWorkingStore,
    synchronize_live_stage,
};
use super::fixtures::response;

#[test]
fn profiles_server_delivery_isolated_overhead() -> Result<()> {
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
            point3f[] points = [(0, 0, 0), (0, 1, 0), (0, 0, 1)]
            int[] faceVertexCounts = [3]
            int[] faceVertexIndices = [0, 1, 2]
        }
    }
}
"#,
    );
    let temp_dir = tempfile::tempdir()?;

    // 1. Measure without RenderServerInterface (Native Local Mode)
    let mut timings_no_server = Vec::new();
    for _ in 0..20 {
        let stage = usd_bevy::UsdSnippet::new(&usda).open_stage().unwrap();
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
        let _ = response(app.world().resource::<SemanticWorkingStore>());

        // Resync /World/A
        app.world()
            .get_non_send::<LiveStage>()
            .unwrap()
            .load_payload("/World/A");
        let t0 = std::time::Instant::now();
        app.update();
        let _ = response(app.world().resource::<SemanticWorkingStore>());
        timings_no_server.push(t0.elapsed());
    }
    timings_no_server.sort();
    let no_server_count = timings_no_server.len();
    let no_server_sum: std::time::Duration = timings_no_server.iter().sum();
    let no_server_mean = no_server_sum / no_server_count as u32;
    let no_server_median = if no_server_count % 2 == 0 {
        (timings_no_server[no_server_count / 2 - 1] + timings_no_server[no_server_count / 2]) / 2
    } else {
        timings_no_server[no_server_count / 2]
    };

    // 2. Measure with RenderServerInterface (Remote Self-Render Mode)
    let mut timings_with_server = Vec::new();
    for _ in 0..20 {
        let stage = usd_bevy::UsdSnippet::new(&usda).open_stage().unwrap();
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
        let server_interface = RenderServerInterface::default();
        app.insert_resource(server_interface);
        app.world_mut().insert_non_send(live);
        app.add_systems(PostUpdate, synchronize_live_stage);
        app.update();
        let _ = response(app.world().resource::<SemanticWorkingStore>());

        // Resync /World/A
        app.world()
            .get_non_send::<LiveStage>()
            .unwrap()
            .load_payload("/World/A");
        let t0 = std::time::Instant::now();
        app.update();
        let _ = response(app.world().resource::<SemanticWorkingStore>());
        timings_with_server.push(t0.elapsed());
    }
    timings_with_server.sort();
    let with_server_count = timings_with_server.len();
    let with_server_sum: std::time::Duration = timings_with_server.iter().sum();
    let with_server_mean = with_server_sum / with_server_count as u32;
    let with_server_median = if with_server_count % 2 == 0 {
        (timings_with_server[with_server_count / 2 - 1]
            + timings_with_server[with_server_count / 2])
            / 2
    } else {
        timings_with_server[with_server_count / 2]
    };

    println!(
        "\n-----------------------------------------------------------------------------------------"
    );
    println!("Server Delivery Whole-Frame Benchmark:");
    println!(
        "  Local Native Whole-Frame (No Server):    mean = {:?}, median = {:?}",
        no_server_mean, no_server_median
    );
    println!(
        "  Server-Enabled Whole-Frame (Delivery):   mean = {:?}, median = {:?}",
        with_server_mean, with_server_median
    );
    println!(
        "  Estimated Server-Delivery Overhead:      mean = {:?}",
        with_server_mean.saturating_sub(no_server_mean)
    );
    println!(
        "-----------------------------------------------------------------------------------------\n"
    );
    Ok(())
}
