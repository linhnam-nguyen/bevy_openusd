//! Render-server owner for the Project-to-LiveStage handoff.

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use usd_project::{ProjectId, SceneId};

use crate::{
    project::{catalog::manifest_store::ManifestStore, service::ProjectStageMutationQueue},
    viewport::session::StageHandle,
};

/// The render-server process owns this queue resource. Project mutation
/// records are read from the active Project's private cache outbox.
#[derive(Resource, Clone, Default)]
pub(super) struct ProjectStageMutationRuntime(pub(super) ProjectStageMutationQueue);

/// Apply canonical Project composition changes on the thread that owns the
/// actual LiveStage. The normal LiveStage drain/reconcile systems then observe
/// the resulting StageChangeBatch.
pub(super) fn consume_project_stage_mutations(world: &mut World) {
    let Some(stage_path) = world
        .get_resource::<StageHandle>()
        .map(|handle| handle.path.clone())
    else {
        return;
    };
    let Some(project_root) = project_root_for_stage(&stage_path) else {
        return;
    };
    let Ok(manifest) = ManifestStore::read_validated(&project_root) else {
        return;
    };
    let active_project_id: ProjectId = manifest.raw().project_id;
    let Some(active_scene_id) = active_scene_id_for_stage(&stage_path, &project_root) else {
        return;
    };
    let Some(live) = world.get_non_send::<usd_bevy::LiveStage>() else {
        return;
    };
    let queue = world.resource::<ProjectStageMutationRuntime>().0.clone();
    if let Err(error) = queue.apply_for_active_scene(
        live,
        &project_root,
        active_project_id,
        Some(active_scene_id),
    ) {
        bevy::log::warn!("Project stage mutation handoff is waiting for retry: {error:?}");
    }
}

fn project_root_for_stage(stage_path: &Path) -> Option<PathBuf> {
    stage_path
        .ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == ".usdhub"))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

fn active_scene_id_for_stage(stage_path: &Path, project_root: &Path) -> Option<SceneId> {
    let scenes_directory = project_root.join(".usdhub").join("scenes");
    let relative = stage_path.strip_prefix(&scenes_directory).ok()?;
    if relative.components().count() != 1 || relative.extension().is_none_or(|ext| ext != "usda") {
        return None;
    }
    let scene_id = SceneId::parse(relative.file_stem()?.to_str()?).ok()?;
    (crate::project::scene::authoring::scene_path(project_root, scene_id) == stage_path)
        .then_some(scene_id)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let scene_id = SceneId::new_v4();
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
}
