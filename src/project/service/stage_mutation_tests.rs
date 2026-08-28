use std::fs;

use openusd::usd::Stage;
use usd_project::{ModelId, ProjectId, SceneId, SceneMemberId};

use super::*;

fn stage() -> usd_bevy::LiveStage {
    usd_bevy::LiveStage::new(
        Stage::builder()
            .in_memory("project-active-stage.usda")
            .unwrap(),
    )
}

#[test]
fn canonical_project_mutations_reach_live_stage_as_real_references() {
    let directory = tempfile::tempdir().unwrap();
    let project_root = directory.path().join("Project");
    fs::create_dir_all(&project_root).unwrap();
    let project_id = ProjectId::new_v4();
    let scene_id = SceneId::new_v4();
    let parent_scene_id = SceneId::new_v4();
    let placement_id = SceneMemberId::new_v4();
    let model_id = ModelId::new_v4();
    let queue = ProjectStageMutationQueue::default();

    queue
        .submit_for_project(
            &project_root,
            ProjectStageMutation::AdoptScene {
                project_id,
                scene_id,
                parent_scene_id: Some(parent_scene_id),
                placement_id: Some(placement_id),
            },
        )
        .unwrap();
    queue
        .submit_for_project(
            &project_root,
            ProjectStageMutation::PublishModel {
                project_id,
                model_id,
                parent_scene_id: Some(parent_scene_id),
                placement_id: Some(SceneMemberId::new_v4()),
            },
        )
        .unwrap();

    let live = stage();
    assert_eq!(
        queue
            .apply_for_active_scene(&live, &project_root, project_id, Some(parent_scene_id))
            .unwrap(),
        2
    );
    let batch = live
        .drain_change_batch()
        .expect("real Project change batch");
    assert!(batch.has_resync());
    let exported = live.stage.root_layer().export_to_string().unwrap();
    assert!(exported.contains("references"));
    assert!(exported.contains(&scene_id.to_string()));
    assert!(exported.contains(&model_id.to_string()));
    assert!(exported.contains(&placement_id.to_string().replace('-', "")));
    assert!(!exported.contains("/__usdhub/project_"));
    assert_eq!(queue.pending_len_for_project(&project_root), 0);
    assert_eq!(
        queue
            .apply_for_active_scene(&live, &project_root, project_id, Some(parent_scene_id))
            .unwrap(),
        0
    );
}

#[test]
fn inactive_project_outbox_remains_isolated() {
    let directory = tempfile::tempdir().unwrap();
    let first_root = directory.path().join("first");
    let second_root = directory.path().join("second");
    fs::create_dir_all(&first_root).unwrap();
    fs::create_dir_all(&second_root).unwrap();
    let queue = ProjectStageMutationQueue::default();
    let first = ProjectId::new_v4();
    let second = ProjectId::new_v4();
    let first_parent = SceneId::new_v4();
    for (root, project_id) in [(&first_root, first), (&second_root, second)] {
        queue
            .submit_for_project(
                root,
                ProjectStageMutation::CreateScene {
                    project_id,
                    scene_id: SceneId::new_v4(),
                    parent_scene_id: Some(first_parent),
                    placement_id: Some(SceneMemberId::new_v4()),
                },
            )
            .unwrap();
    }

    let live = stage();
    queue
        .apply_for_active_scene(&live, &first_root, first, Some(first_parent))
        .unwrap();
    assert_eq!(queue.pending_len_for_project(&first_root), 0);
    assert_eq!(queue.pending_len_for_project(&second_root), 1);
}

#[test]
fn inactive_scene_outbox_remains_pending_until_that_scene_is_active() {
    let directory = tempfile::tempdir().unwrap();
    let project_root = directory.path().join("Project");
    fs::create_dir_all(&project_root).unwrap();
    let project_id = ProjectId::new_v4();
    let expected_parent = SceneId::new_v4();
    let placement_id = SceneMemberId::new_v4();
    let queue = ProjectStageMutationQueue::default();
    queue
        .submit_for_project(
            &project_root,
            ProjectStageMutation::PublishModel {
                project_id,
                model_id: ModelId::new_v4(),
                parent_scene_id: Some(expected_parent),
                placement_id: Some(placement_id),
            },
        )
        .unwrap();

    let live = stage();
    assert_eq!(
        queue
            .apply_for_active_scene(&live, &project_root, project_id, Some(SceneId::new_v4()),)
            .unwrap(),
        0
    );
    assert_eq!(queue.pending_len_for_project(&project_root), 1);
    assert!(
        !live
            .stage
            .root_layer()
            .export_to_string()
            .unwrap()
            .contains("references")
    );

    assert_eq!(
        queue
            .apply_for_active_scene(&live, &project_root, project_id, Some(expected_parent))
            .unwrap(),
        1
    );
    assert_eq!(queue.pending_len_for_project(&project_root), 0);
}

#[test]
fn root_transition_never_patches_the_active_scene() {
    let directory = tempfile::tempdir().unwrap();
    let project_root = directory.path().join("Project");
    fs::create_dir_all(&project_root).unwrap();
    let queue = ProjectStageMutationQueue::default();
    let project_id = ProjectId::new_v4();
    queue
        .submit_for_project(
            &project_root,
            ProjectStageMutation::CreateScene {
                project_id,
                scene_id: SceneId::new_v4(),
                parent_scene_id: None,
                placement_id: None,
            },
        )
        .unwrap();

    let live = stage();
    assert_eq!(
        queue
            .apply_for_active_scene(&live, &project_root, project_id, None)
            .unwrap(),
        1
    );
    assert_eq!(queue.pending_len_for_project(&project_root), 0);
    assert!(
        !live
            .stage
            .root_layer()
            .export_to_string()
            .unwrap()
            .contains("references")
    );
}

#[test]
fn stage_handoff_outbox_is_under_the_approved_cache_root() {
    let project_root = std::path::Path::new("/tmp/Project");
    assert_eq!(
        outbox_path(project_root),
        project_root
            .join(".usdhub")
            .join("cache")
            .join("project-stage-mutations")
    );
}
