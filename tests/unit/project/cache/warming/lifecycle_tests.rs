use std::sync::Arc;

use tempfile::tempdir;
use usd_project::SceneId;

use super::{SCENE_LIFECYCLE_LOCKS, scene_lifecycle_lock};

#[test]
fn scene_lifecycle_lock_registry_retires_released_keys_without_dropping_live_holders() {
    let directory = tempdir().expect("create lifecycle-lock test directory");
    let project_root = directory.path().to_path_buf();
    let first_scene = SceneId::new_v4();
    let second_scene = SceneId::new_v4();
    let third_scene = SceneId::new_v4();
    let first_key = (project_root.clone(), first_scene);
    let second_key = (project_root.clone(), second_scene);
    let third_key = (project_root.clone(), third_scene);

    let first_lock = scene_lifecycle_lock(&project_root, first_scene);
    let first_weak = Arc::downgrade(&first_lock);
    let second_lock = scene_lifecycle_lock(&project_root, second_scene);
    {
        let locks = SCENE_LIFECYCLE_LOCKS
            .lock()
            .expect("lifecycle locks are not poisoned");
        assert!(locks.contains_key(&first_key));
        assert!(locks.contains_key(&second_key));
    }

    drop(second_lock);
    assert!(first_weak.upgrade().is_some());
    drop(first_lock);
    assert!(first_weak.upgrade().is_none());

    let third_lock = scene_lifecycle_lock(&project_root, third_scene);
    let locks = SCENE_LIFECYCLE_LOCKS
        .lock()
        .expect("lifecycle locks are not poisoned");
    assert!(!locks.contains_key(&first_key));
    assert!(!locks.contains_key(&second_key));
    assert!(locks.contains_key(&third_key));
    drop(locks);
    drop(third_lock);
}
