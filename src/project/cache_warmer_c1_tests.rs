use anyhow::Result;
use tempfile::tempdir;
use usd_project::{ProjectId, ProjectManifestV1, ProjectRoot, SceneId};
use viewport_protocol::RuntimeProfile;

use super::*;

#[test]
fn unrelated_scene_edits_keep_a_sibling_cache_identity_reusable() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Warm Project",
        ProjectRoot::Empty,
        vec![
            usd_project::SceneManifestEntry {
                id: SceneId::new_v4(),
                storage_key: usd_project::StorageKey::new("first").unwrap(),
                display_name: "First".to_owned(),
            },
            usd_project::SceneManifestEntry {
                id: SceneId::new_v4(),
                storage_key: usd_project::StorageKey::new("second").unwrap(),
                display_name: "Second".to_owned(),
            },
        ],
        Vec::new(),
    );
    let first_scene = manifest.scenes[0].id;
    let second_scene = manifest.scenes[1].id;
    crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
        directory.path(),
        &manifest,
    )?;
    let first_path =
        crate::project::scene::authoring::author_scene_atomic(directory.path(), first_scene)?;
    let second_path =
        crate::project::scene::authoring::author_scene_atomic(directory.path(), second_scene)?;
    let first_identity = ProjectCacheIdentity::for_project(
        directory.path(),
        ProjectCacheTarget::Scene {
            id: first_scene.to_string(),
        },
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    let store = ProjectCacheStore::new(directory.path());
    store.publish(&ProjectCacheDescriptor::new(
        first_identity.clone(),
        ProjectCacheState::Partial,
        None,
    )?)?;

    std::fs::write(second_path, b"unrelated sibling edit")?;
    let unchanged_identity = ProjectCacheIdentity::for_project(
        directory.path(),
        ProjectCacheTarget::Scene {
            id: first_scene.to_string(),
        },
        RuntimeProfile::NativeMedium,
        crate::project::cache_hydration::default_project_cache_config_hash(),
    )?;
    assert_eq!(first_identity, unchanged_identity);
    assert!(store.load(&unchanged_identity)?.is_some());
    assert!(first_path.is_file());
    Ok(())
}
#[test]
fn latest_scene_work_supersedes_and_saturation_retries_without_orphaning() {
    let state = queue::LatestWarmState::new();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let root = std::path::PathBuf::from("/tmp/usdhub-i16-latest");
    let scene_a = SceneId::new_v4();
    let scene_b = SceneId::new_v4();
    let key_a = (root.clone(), format!("scene:{scene_a}"));
    let key_b = (root.clone(), format!("scene:{scene_b}"));
    let target = |scene_id: SceneId, generation| WarmTarget {
        target: ProjectCacheTarget::Scene { id: scene_id.to_string() },
        scene_generation: Some(generation),
        build_generation: generation,
    };

    assert!(state.record(key_a.clone(), target(scene_a, 1), &sender));
    assert!(state.record(key_a.clone(), target(scene_a, 2), &sender));
    assert!(state.record(key_b.clone(), target(scene_b, 7), &sender));

    let first = receiver.recv().unwrap();
    assert_eq!(first.key, key_a);
    assert_eq!(state.take_latest(&first.key).unwrap().scene_generation, Some(2));
    state.finish_and_retry(&first.key, Some(&sender));

    let retried = receiver.recv().unwrap();
    assert_eq!(retried.key, key_b);
    assert_eq!(state.take_latest(&retried.key).unwrap().scene_generation, Some(7));
}

#[test]
fn cross_project_overflow_applies_bounded_backpressure_without_orphaning_generation() {
    let state = std::sync::Arc::new(queue::LatestWarmState::new());
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let exact_root = std::path::PathBuf::from("/tmp/usdhub-i19-exact");
    let target = |scene_id: SceneId, generation| WarmTarget {
        target: ProjectCacheTarget::Scene { id: scene_id.to_string() },
        scene_generation: Some(generation),
        build_generation: generation,
    };

    for generation in 1..=queue::WARM_LATEST_CAPACITY as u64 {
        let scene_id = SceneId::new_v4();
        assert!(state.record(
            (exact_root.clone(), format!("scene:{scene_id}")),
            target(scene_id, generation),
            &sender,
        ));
    }
    for index in 0..queue::WARM_DIRTY_PROJECT_CAPACITY {
        let root = std::path::PathBuf::from(format!("/tmp/usdhub-i19-dirty-{index}"));
        let scene_id = SceneId::new_v4();
        assert!(state.record(
            (root, format!("scene:{scene_id}")),
            target(scene_id, 50 + index as u64),
            &sender,
        ));
    }
    assert_eq!(
        state.retained_counts(),
        (queue::WARM_LATEST_CAPACITY, queue::WARM_DIRTY_PROJECT_CAPACITY)
    );

    let fifth_root = std::path::PathBuf::from("/tmp/usdhub-i19-dirty-fifth");
    let fifth_scene = SceneId::new_v4();
    let fifth_key = (fifth_root.clone(), format!("scene:{fifth_scene}"));
    let waiter_state = std::sync::Arc::clone(&state);
    let waiter_sender = sender.clone();
    let waiter_key = fifth_key.clone();
    let waiter = std::thread::spawn(move || {
        waiter_state.record(waiter_key, target(fifth_scene, 77), &waiter_sender)
    });
    std::thread::yield_now();
    assert!(!waiter.is_finished());

    let first = receiver.recv().expect("one exact target was scheduled");
    assert!(state.take_latest(&first.key).is_some());
    assert!(waiter.join().expect("bounded producer waiter completes"));

    let fifth_job = receiver.recv().expect("fifth Project retains exact retry ownership");
    assert_eq!(fifth_job.key, fifth_key);
    let retained = state.take_latest(&fifth_job.key).expect("fifth Project work remains retained");
    assert_eq!(retained.scene_generation, Some(77));
    assert_eq!(
        state.retained_counts(),
        (queue::WARM_LATEST_CAPACITY - 1, queue::WARM_DIRTY_PROJECT_CAPACITY)
    );
}

#[test]
fn wide_scene_overflow_is_bounded_and_recovers_from_authoritative_descriptors() -> Result<()> {
    let directory = tempdir()?;
    let scenes = (0..(queue::WARM_LATEST_CAPACITY + 4))
        .map(|index| usd_project::SceneManifestEntry {
            id: SceneId::new_v4(),
            storage_key: usd_project::StorageKey::new(format!("scene-{index}")).unwrap(),
            display_name: format!("Scene {index}"),
        })
        .collect::<Vec<_>>();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Bounded Warm Project",
        ProjectRoot::Empty,
        scenes.clone(),
        Vec::new(),
    );
    crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
        directory.path(),
        &manifest,
    )?;
    let store = SceneCacheStore::new(directory.path());
    let config = crate::project::cache_hydration::default_project_cache_config_hash();
    for (index, scene) in scenes.iter().enumerate() {
        store.publish_descriptor(&SceneCacheDescriptorV3::invalidated(
            scene.id,
            (index + 1) as u64,
            config,
        ))?;
    }

    let state = queue::LatestWarmState::new();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    for (index, scene) in scenes.iter().enumerate() {
        let key = (directory.path().to_path_buf(), format!("scene:{}", scene.id));
        assert!(state.record(
            key,
            WarmTarget {
                target: ProjectCacheTarget::Scene { id: scene.id.to_string() },
                scene_generation: Some((index + 1) as u64),
                build_generation: (index + 1) as u64,
            },
            &sender,
        ));
    }
    let (retained, dirty_projects) = state.retained_counts();
    assert!(retained <= queue::WARM_LATEST_CAPACITY + 1);
    assert_eq!(dirty_projects, 1);

    let mut observed = std::collections::HashSet::new();
    while observed.len() < scenes.len() {
        let job = receiver.recv().expect("bounded scheduler remains live");
        let target = state.take_latest(&job.key).expect("scheduled key has latest work");
        let ProjectCacheTarget::Scene { id } = target.target else { panic!("Scene retry expected") };
        let scene_id = SceneId::parse(&id)?;
        observed.insert(scene_id);
        let mut descriptor = store.load_descriptor(scene_id)?.expect("descriptor remains authoritative");
        descriptor.state = SceneCacheState::Partial;
        store.publish_descriptor(&descriptor)?;
        state.finish_and_retry(&job.key, Some(&sender));
    }
    assert_eq!(observed.len(), scenes.len());
    assert_eq!(state.retained_counts(), (0, 0));
    Ok(())
}
