//! Project filesystem layout, ignore rules, legacy migration, and durability.

use std::path::PathBuf;

use usd_project::SceneId;

mod ignore;
mod layout;
mod migration;

#[cfg(test)]
#[path = "../../../tests/unit/project/storage/portability_tests.rs"]
mod portability_tests;

pub(crate) use ignore::{
    IgnoreChange, has_broad_usdhub_ignore, install_managed_ignore, read_gitignore,
    restore_gitignore,
};
pub(crate) use layout::{
    CACHE_DIRECTORY, CACHE_OBJECTS_RELATIVE_PATH, PROJECT_METADATA_DIRECTORY, ProjectStorageLayout,
    RECOVERY_DIRECTORY, authored_relative_asset_path, authored_relative_project_asset_path,
};
pub(crate) use migration::{
    MigrationPublishCheckpoint, MigrationPublishObserver, NoopMigrationPublishObserver,
    ensure_legacy_migration_supported, migrate_legacy_project_with_observer,
    recover_interrupted_migration,
};

const SCENE_CACHE_DIRECTORY: &str = "scenes";
const SCENE_CACHE_DESCRIPTOR_FILE: &str = "descriptor.json";
const SCENE_CACHE_INDEX_FILE: &str = "index.bin";
const SCENE_CACHE_SPATIAL_FILE: &str = "spatial.bin";
const PROJECT_CACHE_INDEX_FILE: &str = "project-index.bin";
const SHARED_CACHE_OBJECTS_DIRECTORY: &str = "shared";
const CACHE_REVISIONS_DIRECTORY: &str = "revisions";
const CACHE_PINS_DIRECTORY: &str = "pins";
const CACHE_PENDING_DIRECTORY: &str = "pending";
const CACHE_TOMBSTONES_DIRECTORY: &str = "tombstones";
const CACHE_LOCKS_DIRECTORY: &str = ".locks";

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

    pub(crate) fn shared_cache_objects_dir(&self) -> PathBuf {
        self.cache_objects_dir().join(SHARED_CACHE_OBJECTS_DIRECTORY)
    }

    pub(crate) fn cache_revisions_dir(&self) -> PathBuf {
        self.cache_dir().join(CACHE_REVISIONS_DIRECTORY)
    }

    pub(crate) fn cache_pins_dir(&self) -> PathBuf {
        self.cache_dir().join(CACHE_PINS_DIRECTORY)
    }

    pub(crate) fn cache_pending_dir(&self) -> PathBuf {
        self.cache_dir().join(CACHE_PENDING_DIRECTORY)
    }

    pub(crate) fn cache_tombstones_dir(&self) -> PathBuf {
        self.cache_dir().join(CACHE_TOMBSTONES_DIRECTORY)
    }

    pub(crate) fn cache_locks_dir(&self) -> PathBuf {
        self.cache_dir().join(CACHE_LOCKS_DIRECTORY)
    }

    pub(crate) fn project_cache_index_path(&self) -> PathBuf {
        self.cache_dir().join(PROJECT_CACHE_INDEX_FILE)
    }
}
