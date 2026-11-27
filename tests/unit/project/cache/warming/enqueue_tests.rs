use std::sync::{Arc, Mutex, atomic::AtomicU64, mpsc};

use anyhow::Result;
use tempfile::tempdir;
use usd_project::SceneId;

use super::super::{LatestWarmState, ProjectCacheWarmQueue, WarmQueueState};
use super::{ProjectCacheTarget, SceneCacheStore};

#[test]
fn managed_mutation_establishes_all_scene_boundaries_before_admitting_warm_work() -> Result<()> {
    let directory = tempdir()?;
    let first_scene = SceneId::new_v4();
    let second_scene = SceneId::new_v4();
    let (sender, receiver) = mpsc::sync_channel(4);
    let latest = Arc::new(LatestWarmState::new());
    let queue = ProjectCacheWarmQueue {
        state: Arc::new(WarmQueueState {
            sender: Arc::new(Mutex::new(Some(sender))),
            latest: latest.clone(),
            worker: Mutex::new(None),
            next_build_generation: AtomicU64::new(1),
        }),
    };

    let result = queue.enqueue_targets_for_mutation(
        directory.path(),
        vec![
            ProjectCacheTarget::Scene {
                id: first_scene.to_string(),
            },
            ProjectCacheTarget::Scene {
                id: second_scene.to_string(),
            },
            ProjectCacheTarget::Scene {
                id: "not-a-scene-id".to_owned(),
            },
        ],
    );

    assert!(
        result.is_err(),
        "invalid later Scene must remain a fatal managed boundary error"
    );
    assert!(
        receiver.try_recv().is_err(),
        "advisory work must wait for every strict boundary"
    );
    assert_eq!(latest.retained_counts(), (0, 0));
    let store = SceneCacheStore::new(directory.path());
    assert_eq!(
        store
            .load_descriptor(first_scene)?
            .expect("first boundary")
            .generation,
        1
    );
    assert_eq!(
        store
            .load_descriptor(second_scene)?
            .expect("second boundary")
            .generation,
        1
    );
    Ok(())
}

#[test]
fn recover_pending_intents_preserves_durable_intents_when_sender_lock_is_poisoned() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = SceneId::new_v4();
    let manifest = usd_project::ProjectManifestV1::new(
        usd_project::ProjectId::new_v4(),
        "Poisoned Recovery Sender",
        usd_project::ProjectRoot::Scene(scene_id),
        vec![usd_project::SceneManifestEntry {
            id: scene_id,
            storage_key: usd_project::StorageKey::new("root")?,
            display_name: "Root".to_owned(),
        }],
        Vec::new(),
    );
    crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
        directory.path(),
        &manifest,
    )?;
    let store = SceneCacheStore::new(directory.path());
    let generation = store.advance_generation(
        scene_id,
        crate::cache::hydration::default_project_cache_config_hash(),
    )?;
    let target = ProjectCacheTarget::Scene {
        id: scene_id.to_string(),
    };

    let (sender, _receiver) = mpsc::sync_channel(4);
    let sender_lock = Arc::new(Mutex::new(Some(sender)));
    let poison_target = Arc::clone(&sender_lock);
    let _ = std::thread::spawn(move || {
        let _guard = poison_target.lock().expect("acquire sender lock");
        panic!("intentional sender lock poison for recovery test");
    })
    .join();
    assert!(sender_lock.is_poisoned());

    let queue = ProjectCacheWarmQueue {
        state: Arc::new(WarmQueueState {
            sender: sender_lock,
            latest: Arc::new(LatestWarmState::new()),
            worker: Mutex::new(None),
            next_build_generation: AtomicU64::new(1),
        }),
    };

    assert_eq!(
        queue.recover_pending_intents(directory.path()),
        0,
        "poisoned sender lock must fail closed without panicking"
    );
    let preserved = crate::cache::sync::PendingIntentStore::new(directory.path())
        .load(&target)?
        .expect("durable pending intent must remain preserved when sender is poisoned");
    assert_eq!(preserved.generation, generation);
    Ok(())
}
