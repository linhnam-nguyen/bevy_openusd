use super::*;

#[test]
fn canonical_stage_install_preserves_cache_presentation_lookup_and_resident_meshes() {
    let project = tempdir().expect("temporary cache-preservation project");
    let scene_id = usd_project::SceneId::new_v4();
    let generation = 7;
    let config_hash = default_project_cache_config_hash();
    let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, generation, config_hash);
    descriptor.state = SceneCacheState::Partial;
    let index = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation,
        entries: Vec::new(),
    };
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id,
        generation,
        entries: Vec::new(),
    };
    let store = SceneCacheStore::new(project.path());
    store
        .publish_generation(&descriptor, &index, &spatial)
        .expect("publish cache-preservation fixture");
    let activation = store
        .load_activation(scene_id)
        .expect("load cache-preservation activation")
        .expect("cache-preservation activation exists");

    let stage_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/stages/mesh_correctness.usda");
    let stage = Stage::open(stage_path.to_string_lossy().as_ref()).expect("open canonical stage");
    let mut world = World::new();
    world.insert_resource(PrimEntities::default());
    world.insert_resource(Spawned::default());
    world.insert_resource(StageInfo::default());
    world.init_resource::<ProjectionSeed>();
    world.init_resource::<CurrentHierarchyProjection>();
    world.init_resource::<SceneResidencyProjection>();
    assert!(install_scene_cache_bootstrap_before_stage_open(
        &mut world,
        project.path(),
        &activation,
    ));

    let key = ScenePayloadKey {
        scene_id,
        blob_hash: HashDigest::new([9; HashDigest::BYTE_LEN]),
    };
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene_id, generation, Vec::new());
    assert!(authority.request_reason(key, ResidencyReason::Selected, generation, 8, 0,));
    let job = authority.begin_next_load().expect("resident cache job");
    assert!(authority.complete_cached_cpu(
        job,
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default()
        ),
    ));
    let mut assets = Assets::<Mesh>::default();
    authority.pump_uploads(&mut assets, None);
    let resident_asset = authority
        .render_asset_id(&key)
        .expect("cache mesh is GPU resident");
    world.insert_resource(assets);
    world.insert_resource(authority);
    world.insert_resource(ProjectCacheLookup::default());

    activate_open_stage_with_cache_context_for_generation(
        &mut world,
        stage_path,
        stage,
        None,
        Some(activation),
        Some(project.path().to_path_buf()),
        Some(scene_id),
        Some(Vec::new()),
        41,
        StagePresentationContext::default(),
        StageInstallMode::ContinueCacheFirst,
    )
    .expect("install canonical Stage after cache bootstrap");

    assert!(world.get_non_send::<LiveStage>().is_some());
    assert!(world.get_resource::<SceneCachePresentation>().is_some());
    assert!(world.get_resource::<SceneCacheOwnershipContext>().is_some());
    assert!(world.get_resource::<ProjectCacheLookup>().is_some());
    assert!(world.get_resource::<SceneResidencyProjection>().is_some());
    assert_eq!(
        world.resource::<ResidencyAuthority>().render_asset_id(&key),
        Some(resident_asset)
    );
}
