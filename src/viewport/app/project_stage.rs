//! Render-server owner for the Project-to-LiveStage handoff.

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use usd_project::ProjectId;

use crate::{
    project::{catalog::manifest_store::ManifestStore, service::ProjectStageMutationQueue},
    viewport::session::StageHandle,
};

/// The render-server process owns this queue resource. Project mutation
/// records are read from the active Project's private runtime outbox.
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
    let Some(live) = world.get_non_send::<usd_bevy::LiveStage>() else {
        return;
    };
    let queue = world.resource::<ProjectStageMutationRuntime>().0.clone();
    if let Err(error) = queue.apply_for_active_project(live, &project_root, active_project_id) {
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
}
