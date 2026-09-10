use std::path::PathBuf;

use usd_project::SceneId;

#[path = "storage/ignore.rs"]
mod ignore;
#[path = "storage/layout.rs"]
mod layout;
#[path = "storage/migration.rs"]
mod migration;

#[cfg(test)]
#[path = "storage/portability_tests.rs"]
mod portability_tests;

pub(crate) use ignore::{
    IgnoreChange, has_broad_usdhub_ignore, install_managed_ignore, read_gitignore,
    restore_gitignore,
};
pub(crate) use layout::{
    CACHE_DIRECTORY, CACHE_OBJECTS_RELATIVE_PATH, PROJECT_METADATA_DIRECTORY, ProjectStorageLayout,
    RECOVERY_DIRECTORY, authored_relative_asset_path, authored_relative_project_asset_path,
};
pub(crate) use migration::{migrate_legacy_project, recover_interrupted_migration};

const SCENE_CACHE_DIRECTORY: &str = "scenes";
const SCENE_CACHE_DESCRIPTOR_FILE: &str = "descriptor.json";
const SCENE_CACHE_INDEX_FILE: &str = "index.bin";
const SCENE_CACHE_SPATIAL_FILE: &str = "spatial.bin";
const PROJECT_CACHE_INDEX_FILE: &str = "project-index.bin";

impl ProjectStorageLayout {
    pub(crate) fn scene_cache_dir(&self, scene_id: SceneId) -> PathBuf {
        self.cache_dir()
            .join(SCENE_CACHE_DIRECTORY)
            .join(scene_id.to_string())
    }

    pub(crate) fn scene_cache_descriptor_path(&self, scene_id: SceneId) -> PathBuf {
        self.scene_cache_dir(scene_id)
            .join(SCENE_CACHE_DESCRIPTOR_FILE)
    }

    pub(crate) fn scene_cache_index_path(&self, scene_id: SceneId) -> PathBuf {
        self.scene_cache_dir(scene_id).join(SCENE_CACHE_INDEX_FILE)
    }

    pub(crate) fn scene_cache_spatial_path(&self, scene_id: SceneId) -> PathBuf {
        self.scene_cache_dir(scene_id)
            .join(SCENE_CACHE_SPATIAL_FILE)
    }

    pub(crate) fn scene_cache_objects_dir(&self, scene_id: SceneId) -> PathBuf {
        self.cache_objects_dir().join(scene_id.to_string())
    }

    pub(crate) fn project_cache_index_path(&self) -> PathBuf {
        self.cache_dir().join(PROJECT_CACHE_INDEX_FILE)
    }
}
