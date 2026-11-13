//! Test-only helpers attached to the owning production module.
use super::*;

impl SelectionResidencyState {
    pub(super) fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub(super) fn last_batch_work(&self) -> usize {
        self.last_batch_work
    }
}

pub(super) fn build_lookup(
    presentation: &SceneCachePresentation,
    context: Option<&ActiveProjectCacheContext>,
) -> Option<ProjectCacheLookup> {
    let mut entries = presentation.entries.clone();
    entries.sort_by(|left, right| left.address.cmp(&right.address));
    let active = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id: presentation.scene_id,
        generation: presentation.generation,
        entries,
    };
    let mut indexes = vec![active];
    if let Some(context) = context
        && let Ok(manifest) = ManifestStore::read_validated(&context.project_root)
    {
        let store = SceneCacheStore::new(&context.project_root);
        for scene in manifest.scenes() {
            if scene.id == presentation.scene_id {
                continue;
            }
            if let Ok(Some(index)) = store.load_index(scene.id) {
                indexes.push(index);
            }
        }
    }
    ProjectCacheLookup::from_scene_indexes(&indexes).ok()
}
