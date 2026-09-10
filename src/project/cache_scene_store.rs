//! Scene-owned V3 cache storage and Scene-keyed generation serialization.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, Weak},
};

use anyhow::{Context, Result, ensure};
use usd_model::HashDigest;
use usd_project::SceneId;
use uuid::Uuid;

use super::super::cache_contract::{SceneCacheDescriptorV3, SceneCacheIndex, SceneSpatialIndex};
use super::super::{
    blob_store::FilesystemBlobStore, catalog::manifest_store::write_bytes_atomic,
    storage::ProjectStorageLayout,
};

#[derive(Clone, Debug)]
pub(crate) struct SceneCacheStore {
    layout: ProjectStorageLayout,
}

static SCENE_GENERATION_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn scene_generation_lock(path: PathBuf) -> Arc<Mutex<()>> {
    let mut locks = SCENE_GENERATION_LOCKS
        .lock()
        .expect("Scene generation lock registry is not poisoned");
    if let Some(lock) = locks.get(&path).and_then(Weak::upgrade) {
        return lock;
    }
    locks.retain(|_, lock| lock.strong_count() > 0);
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path, Arc::downgrade(&lock));
    lock
}

impl SceneCacheStore {
    pub(crate) fn new(project_root: &Path) -> Self {
        Self {
            layout: ProjectStorageLayout::new(project_root),
        }
    }

    pub(crate) fn object_store(&self, scene_id: SceneId) -> Result<FilesystemBlobStore> {
        FilesystemBlobStore::new(self.layout.scene_cache_objects_dir(scene_id))
    }

    pub(crate) fn advance_generation(
        &self,
        scene_id: SceneId,
        config_hash: HashDigest,
    ) -> Result<u64> {
        let scene_lock = scene_generation_lock(self.layout.scene_cache_dir(scene_id));
        let _guard = scene_lock
            .lock()
            .expect("Scene generation lock is not poisoned");
        let generation = match self.load_descriptor(scene_id)? {
            Some(descriptor) => descriptor
                .generation
                .checked_add(1)
                .context("Scene cache generation overflow")?,
            None => 1,
        };
        self.publish_descriptor(&SceneCacheDescriptorV3::invalidated(
            scene_id,
            generation,
            config_hash,
        ))?;
        Ok(generation)
    }

    /// Publish a Building descriptor when the normal generation read is
    /// available and the next generation is representable. Read and overflow
    /// errors are returned so recovery can remove the Scene cache instead of
    /// manufacturing a non-monotonic boundary.
    pub(crate) fn force_invalidate_generation(
        &self,
        scene_id: SceneId,
        config_hash: HashDigest,
    ) -> Result<()> {
        let scene_lock = scene_generation_lock(self.layout.scene_cache_dir(scene_id));
        let _guard = scene_lock
            .lock()
            .expect("Scene generation lock is not poisoned");
        let generation = match self.load_descriptor(scene_id)? {
            Some(descriptor) => descriptor
                .generation
                .checked_add(1)
                .context("Scene cache generation overflow")?,
            None => 1,
        };
        self.publish_descriptor(&SceneCacheDescriptorV3::invalidated(
            scene_id,
            generation,
            config_hash,
        ))?;
        Ok(())
    }

    /// Remove all disposable cache state when the authoritative manifest is
    /// unavailable and no complete Scene membership list can be derived.
    pub(crate) fn remove_all_derived_cache(&self) -> Result<()> {
        let cache_dir = self.layout.cache_dir();
        for path in [
            cache_dir.join("scenes"),
            self.layout.cache_objects_dir(),
            cache_dir.join("descriptors"),
        ] {
            remove_directory_if_present(&path)?;
        }
        remove_file_if_present(&self.layout.project_cache_index_path())
    }

    pub(crate) fn publish_descriptor(
        &self,
        descriptor: &SceneCacheDescriptorV3,
    ) -> Result<PathBuf> {
        descriptor.validate()?;
        let path = self.layout.scene_cache_descriptor_path(descriptor.scene_id);
        let bytes =
            serde_json::to_vec_pretty(descriptor).context("encode Scene cache descriptor")?;
        self.publish_scene_bytes(descriptor.scene_id, "descriptor", &path, &bytes)?;
        Ok(path)
    }

    pub(crate) fn publish_generation(
        &self,
        descriptor: &SceneCacheDescriptorV3,
        index: &SceneCacheIndex,
        spatial: &SceneSpatialIndex,
    ) -> Result<SceneCacheDescriptorV3> {
        index.validate(descriptor.scene_id, descriptor.generation)?;
        spatial.validate(descriptor.scene_id, descriptor.generation)?;
        let index_bytes = serde_json::to_vec(index).context("encode Scene cache index")?;
        let spatial_bytes = serde_json::to_vec(spatial).context("encode Scene spatial index")?;
        let index_digest = HashDigest::new(*blake3::hash(&index_bytes).as_bytes());
        let spatial_digest = HashDigest::new(*blake3::hash(&spatial_bytes).as_bytes());

        let index_path = self.layout.scene_cache_index_path(descriptor.scene_id);
        self.publish_scene_bytes(descriptor.scene_id, "index", &index_path, &index_bytes)?;
        let spatial_path = self.layout.scene_cache_spatial_path(descriptor.scene_id);
        self.publish_scene_bytes(
            descriptor.scene_id,
            "spatial",
            &spatial_path,
            &spatial_bytes,
        )?;

        let mut published = descriptor.clone();
        published.index_digest = index_digest;
        published.spatial_digest = Some(spatial_digest);
        self.publish_descriptor(&published)?;
        Ok(published)
    }

    pub(crate) fn publish_generation_if_current(
        &self,
        descriptor: &SceneCacheDescriptorV3,
        index: &SceneCacheIndex,
        spatial: &SceneSpatialIndex,
    ) -> Result<Option<SceneCacheDescriptorV3>> {
        let scene_lock = scene_generation_lock(self.layout.scene_cache_dir(descriptor.scene_id));
        let _guard = scene_lock
            .lock()
            .expect("Scene generation lock is not poisoned");
        if !self
            .load_descriptor(descriptor.scene_id)?
            .is_some_and(|current| current.generation == descriptor.generation)
        {
            return Ok(None);
        }
        self.publish_generation(descriptor, index, spatial)
            .map(Some)
    }

    pub(crate) fn remove_scene(&self, scene_id: SceneId) -> Result<()> {
        let scene_lock = scene_generation_lock(self.layout.scene_cache_dir(scene_id));
        let _guard = scene_lock
            .lock()
            .expect("Scene generation lock is not poisoned");
        for path in [
            self.layout.scene_cache_dir(scene_id),
            self.layout.scene_cache_objects_dir(scene_id),
        ] {
            match fs::remove_dir_all(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| format!("remove {}", path.display()));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn load_index(&self, scene_id: SceneId) -> Result<Option<SceneCacheIndex>> {
        let Some(descriptor) = self.load_descriptor(scene_id)? else {
            return Ok(None);
        };
        let path = self.layout.scene_cache_index_path(scene_id);
        let Some(bytes) = self.read_generation_bytes(&path, descriptor.index_digest)? else {
            return Ok(None);
        };
        let index: SceneCacheIndex = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode Scene cache index {}", path.display()))?;
        index.validate(scene_id, descriptor.generation)?;
        Ok(Some(index))
    }

    pub(crate) fn load_spatial(&self, scene_id: SceneId) -> Result<Option<SceneSpatialIndex>> {
        let Some(descriptor) = self.load_descriptor(scene_id)? else {
            return Ok(None);
        };
        let Some(expected) = descriptor.spatial_digest else {
            return Ok(None);
        };
        let path = self.layout.scene_cache_spatial_path(scene_id);
        let Some(bytes) = self.read_generation_bytes(&path, expected)? else {
            return Ok(None);
        };
        let spatial: SceneSpatialIndex = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode Scene spatial index {}", path.display()))?;
        spatial.validate(scene_id, descriptor.generation)?;
        Ok(Some(spatial))
    }

    pub(crate) fn load_descriptor(
        &self,
        scene_id: SceneId,
    ) -> Result<Option<SceneCacheDescriptorV3>> {
        let path = self.layout.scene_cache_descriptor_path(scene_id);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let descriptor: SceneCacheDescriptorV3 = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode Scene cache descriptor {}", path.display()))?;
        ensure!(
            descriptor.scene_id == scene_id,
            "Scene cache descriptor SceneId does not match lookup"
        );
        descriptor.validate()?;
        Ok(Some(descriptor))
    }

    fn publish_scene_bytes(
        &self,
        scene_id: SceneId,
        label: &str,
        path: &Path,
        bytes: &[u8],
    ) -> Result<()> {
        let parent = path
            .parent()
            .context("Scene cache artifact has no parent directory")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("create Scene cache directory {}", parent.display()))?;
        let temporary = parent.join(format!(".{label}.{}.tmp", Uuid::new_v4()));
        write_bytes_atomic(&temporary, path, bytes)
            .with_context(|| format!("publish Scene {label} for {scene_id}"))
    }

    fn read_generation_bytes(&self, path: &Path, expected: HashDigest) -> Result<Option<Vec<u8>>> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let digest = HashDigest::new(*blake3::hash(&bytes).as_bytes());
        Ok((digest == expected).then_some(bytes))
    }
}

fn remove_directory_if_present(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("remove derived cache directory {}", path.display()))
        }
    }
}

fn remove_file_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("remove derived cache file {}", path.display()))
        }
    }
}
