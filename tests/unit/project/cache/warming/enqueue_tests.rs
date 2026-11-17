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
