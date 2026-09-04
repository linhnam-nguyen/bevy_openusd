use std::fs;

use usd_project::{
    ModelId, ProjectId, SceneId, SceneMember, SceneMemberId, SceneMemberTarget,
    ScenePlacementTransform,
};

use super::*;

fn stage(scene_id: SceneId) -> usd_bevy::LiveStage {
    usd_bevy::LiveStage::new(crate::project::scene::authoring::new_scene_stage(scene_id).unwrap())
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
                placement: Some(SceneMember {
                    id: placement_id,
                    target: SceneMemberTarget::Scene(scene_id),
                    name: Some("Adopted Scene".to_owned()),
                    transform: ScenePlacementTransform::from_translation([3.0, 4.0, 5.0]),
                }),
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
                placement: Some(SceneMember {
                    id: SceneMemberId::new_v4(),
                    target: SceneMemberTarget::Model(model_id),
                    name: Some("Published Model".to_owned()),
                    transform: ScenePlacementTransform::from_translation([6.0, 7.0, 8.0]),
                }),
            },
        )
        .unwrap();

    let mut live = stage(parent_scene_id);
    assert_eq!(
        queue
            .apply_for_active_scene(&mut live, &project_root, project_id, Some(parent_scene_id))
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
    let active_stage_path = directory.path().join("active-stage.usda");
    live.stage
        .root_layer()
        .export(active_stage_path.to_string_lossy().as_ref())
        .unwrap();
    let active_members =
        crate::project::scene::authoring::read_scene_members(&active_stage_path, parent_scene_id)
            .unwrap();
    let adopted_member = active_members
        .iter()
        .find(|member| member.id == placement_id)
        .expect("adopted placement is present in the active stage");
    assert_eq!(adopted_member.name.as_deref(), Some("Adopted Scene"));
    assert_eq!(
        adopted_member.transform,
        ScenePlacementTransform::from_translation([3.0, 4.0, 5.0])
    );
    assert_eq!(queue.pending_len_for_project(&project_root), 0);
    assert_eq!(
        queue
            .apply_for_active_scene(&mut live, &project_root, project_id, Some(parent_scene_id))
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
        let scene_id = SceneId::new_v4();
        queue
            .submit_for_project(
                root,
                ProjectStageMutation::CreateScene {
                    project_id,
                    scene_id,
                    parent_scene_id: Some(first_parent),
                    placement: Some(SceneMember {
                        id: SceneMemberId::new_v4(),
                        target: SceneMemberTarget::Scene(scene_id),
                        name: None,
                        transform: ScenePlacementTransform::IDENTITY,
                    }),
                },
            )
            .unwrap();
    }

    let mut live = stage(first_parent);
    queue
        .apply_for_active_scene(&mut live, &first_root, first, Some(first_parent))
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
    let model_id = ModelId::new_v4();
    let queue = ProjectStageMutationQueue::default();
    queue
        .submit_for_project(
            &project_root,
            ProjectStageMutation::PublishModel {
                project_id,
                model_id,
                parent_scene_id: Some(expected_parent),
                placement: Some(SceneMember {
                    id: placement_id,
                    target: SceneMemberTarget::Model(model_id),
                    name: Some("Pending Model".to_owned()),
                    transform: ScenePlacementTransform::IDENTITY,
                }),
            },
        )
        .unwrap();

    let mut live = stage(expected_parent);
    assert_eq!(
        queue
            .apply_for_active_scene(
                &mut live,
                &project_root,
                project_id,
                Some(SceneId::new_v4()),
            )
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
            .apply_for_active_scene(&mut live, &project_root, project_id, Some(expected_parent))
            .unwrap(),
        1
    );
    assert_eq!(queue.pending_len_for_project(&project_root), 0);
}

#[test]
fn deleting_an_inactive_scene_is_consumed_without_mutating_the_active_stage() {
    let directory = tempfile::tempdir().unwrap();
    let project_root = directory.path().join("Project");
    fs::create_dir_all(&project_root).unwrap();
    let project_id = ProjectId::new_v4();
    let deleted_scene_id = SceneId::new_v4();
    let active_scene_id = SceneId::new_v4();
    let queue = ProjectStageMutationQueue::default();
    queue
        .submit_for_project(
            &project_root,
            ProjectStageMutation::DeleteScene {
                project_id,
                scene_id: deleted_scene_id,
            },
        )
        .unwrap();

    let mut live = stage(active_scene_id);
    let before = live.stage.root_layer().export_to_string().unwrap();
    assert_eq!(
        queue
            .apply_for_active_scene(&mut live, &project_root, project_id, Some(active_scene_id))
            .unwrap(),
        1
    );
    assert_eq!(queue.pending_len_for_project(&project_root), 0);
    let after = live.stage.root_layer().export_to_string().unwrap();
    assert_eq!(after, before);
    assert!(after.contains("SceneRoot"));
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
                placement: None,
            },
        )
        .unwrap();

    let mut live = stage(SceneId::new_v4());
    assert_eq!(
        queue
            .apply_for_active_scene(&mut live, &project_root, project_id, None)
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

#[test]
fn injected_batch_failure_leaves_no_partial_stage_mutations() {
    let directory = tempfile::tempdir().unwrap();
    let project_root = directory.path().join("Project");
    fs::create_dir_all(&project_root).unwrap();
    let project_id = ProjectId::new_v4();
    let mutations = [
        ProjectStageMutation::DeleteModel {
            project_id,
            model_id: ModelId::new_v4(),
        },
        ProjectStageMutation::DeleteScene {
            project_id,
            scene_id: SceneId::new_v4(),
        },
    ];

    assert!(submit_batch_locked_with_failure(&project_root, &mutations, Some(1)).is_err());
    assert_eq!(outbox_path(&project_root).read_dir().unwrap().count(), 0);
}
