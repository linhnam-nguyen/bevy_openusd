use bevy::prelude::World;

use crate::project::cache_contract::SceneCacheActivation;

pub(crate) fn publish_scene_cache_presentation_before_stage_open(
    world: &mut World,
    project_root: &std::path::Path,
    activation: &SceneCacheActivation,
) -> bool {
    if !is_current(Some(project_root), activation) {
        return false;
    }
    super::scene_presentation::publish_scene_cache_presentation(world, activation, Some(project_root));
    true
}

pub(super) fn is_current(
    project_root: Option<&std::path::Path>,
    activation: &SceneCacheActivation,
) -> bool {
    let Some(project_root) = project_root else { return false; };
    let store = crate::project::cache::SceneCacheStore::new(project_root);
    match store.load_descriptor(activation.descriptor.scene_id) {
        Ok(Some(current)) => {
            current.generation == activation.descriptor.generation
                && current.index_digest == activation.descriptor.index_digest
                && current.state == activation.descriptor.state
        }
        Ok(None) => false,
        Err(error) => {
            bevy::log::warn!(
                "[project-cache] Scene metadata generation check failed for {}: {error:#}",
                activation.descriptor.scene_id
            );
            false
        }
    }
}
