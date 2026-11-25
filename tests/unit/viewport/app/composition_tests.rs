use std::path::PathBuf;

use bevy::prelude::*;
use usd_bevy::{LiveStagePlugin, UsdPlugin};

use super::*;
use crate::api::{ViewportBridgePlugin, ViewportEventOutbox, ViewportReadModelState};
use crate::app::launch_options::parse_launch_options;
use crate::viewport::stage::{RequestedAsset, StageInfo};

fn minimal_host_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        bevy::asset::AssetPlugin::default(),
    ));
    app.init_resource::<Assets<Shader>>();
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<Assets<Image>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.init_resource::<Assets<crate::viewport::rendering::scene::SectionClipMaterial>>();
    app
}

#[test]
fn generic_viewport_wires_without_project_git_turso_or_cache_providers() {
    let mut app = minimal_host_app();

    let config = GenericViewportConfig {
        asset_path: String::new(),
        asset_root: PathBuf::from("assets"),
        fps: 60,
        benchmark_mesh_profile: false,
        skip_initial_stage_load: true,
    };

    configure_generic_viewport_core(&mut app, &config);
    // Calling twice must remain idempotent with unique plugin registration.
    configure_generic_viewport_core(&mut app, &config);

    assert!(app.is_plugin_added::<UsdPlugin>());
    assert!(app.is_plugin_added::<LiveStagePlugin>());
    assert!(!app.is_plugin_added::<ViewportBridgePlugin>());
    assert!(app.world().contains_resource::<StageInfo>());
    assert!(app.world().contains_resource::<RequestedAsset>());
    assert!(app.world().contains_resource::<ViewportEventOutbox>());
    assert!(app.world().contains_resource::<ViewportReadModelState>());
    // Project/Git/Turso/Cache runtime providers must NOT be registered in generic mode.
    assert!(
        !app.world()
            .contains_resource::<crate::semantic::viewport::SemanticSyncRuntimeResource>()
    );
    assert!(
        !app.world()
            .contains_resource::<crate::document::stages::recovery::RecoverySettings>()
    );
    assert!(
        !app.world()
            .contains_resource::<crate::cache::historical_geometry::HistoricalGeometryCache>()
    );

    app.update();

    // C11-01/F-01/F-02R/F-04 production proof: standalone USD source identity
    // (fail-closed) and real NMSH mesh-payload cache wired through production
    // `load_stage` + `LiveStagePlugin` + `publish_standalone_cache_when_ready`
    // + `ProjectionSeed` hydration with ZERO Project, Git, Turso, or Bridge.
    let tmp = tempfile::tempdir().expect("standalone cache tempdir");
    let missing_usda = tmp.path().join("does_not_exist.usda");
    assert!(
        crate::cache::compute_standalone_source_identity(&missing_usda).is_err(),
        "missing USD entry must fail closed"
    );
    let usda = tmp.path().join("standalone_mesh.usda");
    std::fs::write(
        &usda,
        "#usda 1.0\ndef Mesh \"Tri\" {\n  int[] faceVertexCounts = [3]\n  int[] faceVertexIndices = [0, 1, 2]\n  point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]\n}\n",
    )
    .expect("write standalone mesh usda");
    let cache_dir = tmp.path().join("cache");
    let store = crate::cache::StandaloneCacheStore::new(&cache_dir);
    let recipe = crate::cache::generic::default_standalone_recipe_digest();

    // 1st production open (MISS): projects real triangle mesh from USD and
    // publishes real NMSH-encoded mesh payload via `publish_standalone_cache_when_ready`.
    let mut first_app = minimal_host_app();
    let first_cfg = GenericViewportConfig {
        asset_path: "standalone_mesh.usda".to_owned(),
        asset_root: tmp.path().to_path_buf(),
        fps: 60,
        benchmark_mesh_profile: false,
        skip_initial_stage_load: false,
    };
    configure_generic_viewport_core(&mut first_app, &first_cfg);
    first_app.insert_resource(crate::cache::generic::StandaloneCacheProviderResource {
        store: store.clone(),
        recipe_digest: recipe,
    });
    for _ in 0..4 {
        first_app.update();
    }
    let published = first_app
        .world()
        .get_resource::<crate::cache::generic::ActiveStandaloneCacheHandle>()
        .expect("1st production open must publish real mesh cache")
        .0
        .clone();
    assert!(!published.hydrated_from_cache, "1st open was a cache miss");
    assert_eq!(published.meshes.len(), 1, "1 real triangle mesh cached");

    // 2nd production open in a fresh standalone app (HIT): hydrates decoded
    // real Mesh into `usd_bevy::ProjectionSeed`, `LiveStagePlugin` consumes the
    // seed, and the projected `/Tri` entity carries the 3-vertex mesh.
    let mut second_app = minimal_host_app();
    configure_generic_viewport_core(&mut second_app, &first_cfg);
    second_app.insert_resource(crate::cache::generic::StandaloneCacheProviderResource {
        store: store.clone(),
        recipe_digest: recipe,
    });
    for _ in 0..4 {
        second_app.update();
    }
    let hydrated = second_app
        .world()
        .get_resource::<crate::cache::generic::ActiveStandaloneCacheHandle>()
        .expect("2nd production open must hit real mesh cache")
        .0
        .clone();
    assert!(hydrated.hydrated_from_cache, "2nd open must hydrate from cache");
    assert_eq!(hydrated.meshes, published.meshes);
    assert_eq!(
        second_app
            .world()
            .resource::<usd_bevy::ProjectionSeed>()
            .pending_meshes(),
        0,
        "LiveStagePlugin must consume the hydrated ProjectionSeed mesh"
    );
    let projected_vertices = {
        let prim_entities = second_app.world().resource::<usd_bevy::PrimEntities>();
        let path_store = second_app.world().resource::<usd_bevy::PathStore>();
        let mesh_assets = second_app.world().resource::<Assets<Mesh>>();
        let tri_entity = prim_entities
            .iter(path_store)
            .find_map(|(path, entity)| (path == "/Tri").then_some(entity))
            .expect("2nd production open must project /Tri entity");
        let mesh3d = second_app
            .world()
            .get::<Mesh3d>(tri_entity)
            .expect("projected /Tri entity must have Mesh3d");
        mesh_assets
            .get(&mesh3d.0)
            .expect("projected Mesh3d handle must resolve in Assets<Mesh>")
            .count_vertices()
    };
    assert_eq!(projected_vertices, 3, "projected /Tri mesh must have 3 vertices");

    // Negative (F-02R-SAFE): stale authoring_generation on pending publish must
    // be discarded without publishing, and mutating the USD file on disk must
    // miss the old key and publish a distinct 4-vertex mesh under the new key.
    let stale_cache_dir = tmp.path().join("stale_guard_cache");
    let stale_store = crate::cache::StandaloneCacheStore::new(&stale_cache_dir);
    let source_id = crate::cache::compute_standalone_source_identity(&usda)
        .expect("compute standalone source identity");
    let cache_id = crate::cache::CacheIdentity::new(source_id.clone(), recipe);
    let stage_identity = second_app
        .world()
        .get_non_send::<usd_bevy::LiveStage>()
        .expect("active LiveStage")
        .stage_identity();
    let generation = second_app
        .world()
        .resource::<StageInfo>()
        .activation_generation;
    second_app.insert_resource(crate::cache::generic::StandaloneCacheProviderResource {
        store: stale_store.clone(),
        recipe_digest: recipe,
    });
    second_app.insert_resource(crate::cache::generic::PendingStandaloneCachePublish {
        identity: cache_id.clone(),
        stage_identity,
        authoring_generation: usd_bevy::LiveRevision(999_999),
        generation,
    });
    second_app.update();
    assert!(
        !second_app
            .world()
            .contains_resource::<crate::cache::generic::PendingStandaloneCachePublish>(),
        "mismatched authoring_generation must discard PendingStandaloneCachePublish"
    );
    assert!(
        !stale_store.descriptor_path(&cache_id).exists(),
        "mismatched authoring_generation must not publish descriptor"
    );

    // Mutate the USD file on disk to a 4-vertex quad and open a 3rd app:
    // must miss the 3-vertex key and publish a fresh 4-vertex mesh under the new key.
    std::fs::write(
        &usda,
        "#usda 1.0\ndef Mesh \"Tri\" {\n  int[] faceVertexCounts = [4]\n  int[] faceVertexIndices = [0, 1, 2, 3]\n  point3f[] points = [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]\n}\n",
    )
    .expect("mutate standalone mesh usda");
    let mut third_app = minimal_host_app();
    configure_generic_viewport_core(&mut third_app, &first_cfg);
    third_app.insert_resource(crate::cache::generic::StandaloneCacheProviderResource {
        store: store.clone(),
        recipe_digest: recipe,
    });
    for _ in 0..4 {
        third_app.update();
    }
    let mutated_published = third_app
        .world()
        .get_resource::<crate::cache::generic::ActiveStandaloneCacheHandle>()
        .expect("3rd production open must publish mutated mesh under new key")
        .0
        .clone();
    assert!(
        !mutated_published.hydrated_from_cache,
        "mutated USD file must miss prior cache entry"
    );
    assert_ne!(
        mutated_published.descriptor_path, published.descriptor_path,
        "mutated USD file must write to a distinct storage key"
    );
    let mutated_source_id = crate::cache::compute_standalone_source_identity(&usda)
        .expect("compute mutated standalone source identity");
    let mutated_cache_id = crate::cache::CacheIdentity::new(mutated_source_id, recipe);
    let (_, mutated_decoded) = store
        .hydrate_meshes(&mutated_cache_id)
        .expect("hydrate mutated meshes")
        .expect("mutated mesh hit");
    assert_eq!(mutated_decoded.len(), 1);
    assert_eq!(mutated_decoded[0].1.count_vertices(), 4);

    // Negative: verify original 3-vertex decoded mesh, then test
    // corrupt blob and corrupt descriptor on the original cache entry.
    let (_, decoded) = store
        .hydrate_meshes(&cache_id)
        .expect("hydrate_meshes")
        .expect("cached mesh hit");
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].1.count_vertices(), 3);

    // Same-length / content corruption of the .blob file must invalidate hit.
    let hex = published.meshes[0].blob_id.0.clone();
    let blob_path = published.blob_root.join(&hex[..2]).join(format!("{hex}.blob"));
    assert!(blob_path.is_file(), "published NMSH mesh blob must exist");
    let orig_len = std::fs::metadata(&blob_path).expect("blob metadata").len() as usize;
    std::fs::write(&blob_path, vec![b'X'; orig_len]).expect("same-length corrupt blob");
    assert!(
        store
            .hydrate_meshes(&cache_id)
            .expect("corrupt blob falls back to miss")
            .is_none()
    );

    // Corrupt descriptor JSON must fall back to miss.
    std::fs::write(&published.descriptor_path, b"{not-valid-json")
        .expect("corrupt descriptor");
    assert!(
        store
            .hydrate_meshes(&cache_id)
            .expect("corrupt descriptor falls back to miss")
            .is_none()
    );
}

#[test]
fn shared_application_composition_preserves_launch_flags_and_injects_product_services() {
    let options = parse_launch_options([
        "--headless".to_string(),
        "--fps".to_string(),
        "30".to_string(),
        "--width".to_string(),
        "1280".to_string(),
        "--height".to_string(),
        "720".to_string(),
    ])
    .expect("valid headless launch options");

    let mut app = minimal_host_app();

    let generic_config = GenericViewportConfig {
        asset_path: String::new(),
        asset_root: PathBuf::from("assets"),
        fps: options.fps,
        benchmark_mesh_profile: options.benchmark_mesh_profile,
        skip_initial_stage_load: true,
    };
    configure_generic_viewport_core(&mut app, &generic_config);
    configure_product_services(&mut app, &options, "", true);

    assert!(app.is_plugin_added::<ViewportBridgePlugin>());
    assert!(
        app.world()
            .contains_resource::<crate::document::stages::recovery::RecoverySettings>()
    );
    assert_eq!(
        app.world()
            .resource::<RendererCadence>()
            .effective_renderer_target_fps(),
        Some(30)
    );

    app.update();
    let mut outbox = app.world_mut().resource_mut::<ViewportEventOutbox>();
    let mut saw_ready = false;
    while let Some(envelope) = outbox.pop() {
        if matches!(envelope.event, viewport_protocol::ViewportEvent::Ready { .. }) {
            saw_ready = true;
        }
    }
    assert!(saw_ready, "ViewportBridgePlugin emits protocol Ready on Startup");
}
