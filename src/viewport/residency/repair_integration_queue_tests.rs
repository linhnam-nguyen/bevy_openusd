use super::*;

#[test]
fn waiting_key_rotates_while_ready_key_reaches_real_persistence() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let scene = SceneId::new_v4();
    let ready_mesh = mesh();
    let ready_hash = hash_for(&ready_mesh);
    let waiting_hash = HashDigest::new([20; HashDigest::BYTE_LEN]);
    let index = owner_index_for_entries(
        scene,
        1,
        vec![
            (waiting_hash, "/SceneRoot/Waiting"),
            (ready_hash, "/SceneRoot/Ready"),
        ],
    );
    let mut descriptor =
        SceneCacheDescriptorV3::invalidated(scene, 1, HashDigest::new([17; HashDigest::BYTE_LEN]));
    descriptor.state = SceneCacheState::Partial;
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id: scene,
        generation: 1,
        entries: Vec::new(),
    };
    let expected =
        SceneCacheStore::new(directory.path()).publish_generation(&descriptor, &index, &spatial)?;
    let waiting = load_job(scene, waiting_hash, 1);
    let ready = load_job(scene, ready_hash, 1);
    let mut authority = waiting_authority(&waiting);
    assert!(authority.request_reason(
        ready.key,
        ResidencyReason::CameraNear,
        1,
        ready.cpu_bytes,
        ready.gpu_bytes,
    ));
    let ready_loading = authority.begin_next_load().expect("ready job starts");
    assert!(authority.wait_for_repair(&ready_loading));
    let mut repairs = TargetedRepairQueue::default();
    assert!(repairs.enqueue(repair_request(directory.path(), waiting)));
    assert!(repairs.enqueue(TargetedRepairRequest {
        project_root: Some(directory.path().to_path_buf()),
        path: Some("/SceneRoot/Ready".to_owned()),
        extraction: Some(TargetedSceneRepair {
            expected_descriptor: expected,
            payloads: vec![usd_bevy::TargetedRenderPayload {
                path: "/SceneRoot/Ready".to_owned(),
                mesh: ready_mesh,
                local_bounds: None,
            }],
        }),
        lookup_only: false,
        phase: RepairPhase::NeedScenePayload,
        job: ready_loading.clone(),
    }));

    let mut world = World::new();
    world.insert_resource(context(directory.path()));
    world.insert_resource(TargetedRepairPersistenceWorker::new());
    world.insert_resource(authority);
    world.insert_resource(repairs);
    process_once(&mut world);
    assert_eq!(world.resource::<TargetedRepairQueue>().len(), 1);
    let completion = wait_persistence(world.resource::<TargetedRepairPersistenceWorker>());
    assert_eq!(completion.job.key, ready.key);
    assert!(matches!(
        completion.result,
        Ok(TargetedPersistenceOutcome::Published { .. })
    ));
    Ok(())
}
