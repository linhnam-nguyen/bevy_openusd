use std::path::{Path, PathBuf};

use super::{active_scene_id_for_stage, project_root_for_stage};

#[test]
fn project_root_is_derived_only_for_a_project_scene_path() {
    let path = Path::new("/tmp/Project/.usdhub/scenes/scene.usda");
    assert_eq!(
        project_root_for_stage(path),
        Some(PathBuf::from("/tmp/Project"))
    );
    assert!(project_root_for_stage(Path::new("/tmp/scene.usda")).is_none());
}

#[test]
fn active_scene_identity_requires_the_canonical_scene_locator() {
    let project_root = Path::new("/tmp/Project");
    let scene_id = usd_project::SceneId::new_v4();
    let canonical = crate::project::scene::authoring::scene_path(project_root, scene_id);
    assert_eq!(
        active_scene_id_for_stage(&canonical, project_root),
        Some(scene_id)
    );
    assert!(
        active_scene_id_for_stage(
            &project_root.join(".usdhub/scenes/scene.usda"),
            project_root,
        )
        .is_none()
    );
    assert!(
        active_scene_id_for_stage(&project_root.join(".usdhub/project.usda"), project_root,)
            .is_none()
    );
}
