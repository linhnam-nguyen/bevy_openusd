use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use usd_project::{
    ModelId, ModelManifestEntry, ProjectManifestV1, ProjectRoot, SceneId, SceneManifestEntry,
    StorageKey,
};

pub(crate) const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";
pub(crate) const SCENES_DIRECTORY: &str = "scenes";
pub(crate) const MODELS_DIRECTORY: &str = "models";
pub(crate) const CACHE_DIRECTORY: &str = "cache";
pub(crate) const RECOVERY_DIRECTORY: &str = "recovery";
pub(crate) const LINKS_DIRECTORY: &str = "links";
pub(crate) const CACHE_OBJECTS_RELATIVE_PATH: &str = ".usdhub/cache/objects";
pub(crate) const PROJECT_MANIFEST_FILE: &str = "project.json";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectStorageLayout {
    root: PathBuf,
}

impl ProjectStorageLayout {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn metadata_dir(&self) -> PathBuf {
        self.root.join(PROJECT_METADATA_DIRECTORY)
    }

    /// Local, derived, and recoverable state; never canonical identity.
    pub(crate) fn local_state_dir(&self) -> PathBuf {
        self.metadata_dir()
    }

    /// Canonical Git-tracked Project metadata in Storage v2.
    pub(crate) fn canonical_manifest_path(&self) -> PathBuf {
        self.root.join(PROJECT_MANIFEST_FILE)
    }

    /// Pre-Storage-v2 manifest location retained for migration.
    pub(crate) fn legacy_manifest_path(&self) -> PathBuf {
        self.metadata_dir().join(PROJECT_MANIFEST_FILE)
    }

    pub(crate) fn manifest_path(&self) -> PathBuf {
        self.canonical_manifest_path()
    }

    pub(crate) fn readable_manifest_path(&self) -> PathBuf {
        if self.canonical_manifest_path().is_file() {
            self.canonical_manifest_path()
        } else {
            self.legacy_manifest_path()
        }
    }

    pub(crate) fn scenes_dir(&self) -> PathBuf {
        self.metadata_dir().join(SCENES_DIRECTORY)
    }

    pub(crate) fn canonical_scenes_dir(&self) -> PathBuf {
        self.root.join(SCENES_DIRECTORY)
    }

    pub(crate) fn canonical_root_scene_path(&self, storage_key: &StorageKey) -> PathBuf {
        self.root.join(format!("{}.usda", storage_key.as_str()))
    }

    pub(crate) fn canonical_scene_path(&self, storage_key: &StorageKey) -> PathBuf {
        self.canonical_scenes_dir()
            .join(format!("{}.usda", storage_key.as_str()))
    }

    pub(crate) fn scene_path(&self, scene: &SceneManifestEntry) -> PathBuf {
        self.canonical_scene_path(&scene.storage_key)
    }

    pub(crate) fn readable_scene_path(
        &self,
        manifest: &ProjectManifestV1,
        scene: &SceneManifestEntry,
    ) -> PathBuf {
        let canonical = if manifest.root == ProjectRoot::Scene(scene.id) {
            self.canonical_root_scene_path(&scene.storage_key)
        } else {
            self.canonical_scene_path(&scene.storage_key)
        };
        if canonical.is_file() {
            canonical
        } else if manifest.root != ProjectRoot::Scene(scene.id)
            && self.canonical_root_scene_path(&scene.storage_key).is_file()
        {
            // Transitional compatibility for a pre-C3 root Scene that was
            // already named by StorageKey but has not yet moved under scenes/.
            self.canonical_root_scene_path(&scene.storage_key)
        } else {
            self.legacy_scene_path(scene.id)
        }
    }

    pub(crate) fn legacy_scene_path(&self, scene_id: SceneId) -> PathBuf {
        self.scenes_dir().join(format!("{scene_id}.usda"))
    }

    pub(crate) fn models_dir(&self) -> PathBuf {
        self.metadata_dir().join(MODELS_DIRECTORY)
    }

    pub(crate) fn canonical_models_dir(&self) -> PathBuf {
        self.root.join(MODELS_DIRECTORY)
    }

    pub(crate) fn canonical_model_wrapper_path(&self, model: &ModelManifestEntry) -> PathBuf {
        self.canonical_models_dir()
            .join(model.storage_key.as_str())
            .join("model.usda")
    }

    pub(crate) fn legacy_model_wrapper_path(&self, model_id: ModelId) -> PathBuf {
        self.models_dir()
            .join(model_id.to_string())
            .join("model.usda")
    }

    pub(crate) fn imports_dir(&self) -> PathBuf {
        self.root.join("imports")
    }

    pub(crate) fn canonical_scene_import_dir(&self, scene_id: SceneId) -> PathBuf {
        self.imports_dir()
            .join(SCENES_DIRECTORY)
            .join(scene_id.to_string())
    }

    pub(crate) fn readable_scene_import_dir(&self, scene_id: SceneId) -> PathBuf {
        let canonical = self.canonical_scene_import_dir(scene_id);
        if canonical.exists() {
            canonical
        } else {
            self.legacy_scene_import_dir(scene_id)
        }
    }

    pub(crate) fn canonical_model_import_dir(&self, model_id: ModelId) -> PathBuf {
        self.imports_dir()
            .join(MODELS_DIRECTORY)
            .join(model_id.to_string())
    }

    pub(crate) fn readable_model_import_dir(&self, model_id: ModelId) -> PathBuf {
        let canonical = self.canonical_model_import_dir(model_id);
        if canonical.exists() {
            canonical
        } else {
            self.legacy_model_import_dir(model_id)
        }
    }

    pub(crate) fn legacy_scene_import_dir(&self, scene_id: SceneId) -> PathBuf {
        self.metadata_dir()
            .join("imports")
            .join(SCENES_DIRECTORY)
            .join(scene_id.to_string())
    }

    pub(crate) fn legacy_model_import_dir(&self, model_id: ModelId) -> PathBuf {
        self.metadata_dir()
            .join("imports")
            .join(MODELS_DIRECTORY)
            .join(model_id.to_string())
    }

    pub(crate) fn cache_dir(&self) -> PathBuf {
        self.metadata_dir().join(CACHE_DIRECTORY)
    }

    pub(crate) fn cache_objects_dir(&self) -> PathBuf {
        self.cache_dir().join("objects")
    }

    pub(crate) fn recovery_dir(&self) -> PathBuf {
        self.metadata_dir().join(RECOVERY_DIRECTORY)
    }

    pub(crate) fn links_dir(&self) -> PathBuf {
        self.metadata_dir().join(LINKS_DIRECTORY)
    }

    pub(crate) fn ensure_local_state_roots(&self) -> Result<()> {
        fs::create_dir_all(self.cache_dir()).context("create Project cache root")?;
        fs::create_dir_all(self.recovery_dir()).context("create Project recovery root")?;
        Ok(())
    }
}

pub(crate) fn authored_relative_asset_path(
    authoring_layer_path: &Path,
    target_asset_path: &Path,
) -> Result<String> {
    let authoring_layer_path = normalized_path(authoring_layer_path)?;
    let target_asset_path = normalized_path(target_asset_path)?;
    let authoring_parent = authoring_layer_path
        .parent()
        .context("authoring layer has no parent directory")?;
    let relative = lexical_relative(authoring_parent, &target_asset_path)?;
    let text = relative
        .to_str()
        .context("relative USD asset path must be valid UTF-8")?
        .replace('\\', "/");
    if text.is_empty() || Path::new(&text).is_absolute() {
        bail!("USD asset path must be non-empty and relative")
    }
    Ok(text)
}

pub(crate) fn authored_relative_project_asset_path(
    project_root: &Path,
    authoring_layer_path: &Path,
    target_asset_path: &Path,
) -> Result<String> {
    let project_root = normalized_path(project_root)?;
    let authoring_layer_path = normalized_path(authoring_layer_path)?;
    let target_asset_path = normalized_path(target_asset_path)?;
    if !authoring_layer_path.starts_with(&project_root)
        || !target_asset_path.starts_with(&project_root)
    {
        bail!("canonical Project USD asset reference escapes the Project root")
    }
    authored_relative_asset_path(&authoring_layer_path, &target_asset_path)
}

fn normalized_path(path: &Path) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    bail!("path escapes its lexical root: {}", path.display())
                }
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str())
            }
        }
    }
    if normalized.is_absolute() != path.is_absolute() {
        bail!("path normalization changed path kind: {}", path.display())
    }
    Ok(normalized)
}

fn lexical_relative(base: &Path, target: &Path) -> Result<PathBuf> {
    if base.is_absolute() != target.is_absolute() {
        bail!("cannot relativize absolute and relative paths")
    }
    let base = base.components().collect::<Vec<_>>();
    let target = target.components().collect::<Vec<_>>();
    let common = base
        .iter()
        .zip(&target)
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = PathBuf::new();
    for _ in common..base.len() {
        relative.push("..");
    }
    for component in &target[common..] {
        relative.push(component.as_os_str());
    }
    Ok(relative)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn storage_v2_paths_are_keyed_by_stable_storage_identity() {
        let layout = ProjectStorageLayout::new("project");
        let scene = SceneManifestEntry {
            id: SceneId::new_v4(),
            storage_key: StorageKey::new("Lv1").unwrap(),
            display_name: "Level One".to_owned(),
        };
        let model = ModelManifestEntry {
            id: ModelId::new_v4(),
            source_kind: usd_project::ModelSourceKind::Usd,
            storage_key: StorageKey::new("Chair").unwrap(),
            display_name: "Chair".to_owned(),
        };

        assert_eq!(
            layout.canonical_manifest_path(),
            PathBuf::from("project/project.json")
        );
        assert_eq!(
            layout.canonical_root_scene_path(&StorageKey::new("Pro2").unwrap()),
            PathBuf::from("project/Pro2.usda")
        );
        assert_eq!(
            layout.scene_path(&scene),
            PathBuf::from("project/scenes/Lv1.usda")
        );
        assert_eq!(
            layout.canonical_model_wrapper_path(&model),
            PathBuf::from("project/models/Chair/model.usda")
        );
        assert_eq!(
            layout.canonical_scene_import_dir(scene.id),
            PathBuf::from(format!("project/imports/scenes/{}", scene.id))
        );
        assert_eq!(
            layout.legacy_scene_path(scene.id),
            PathBuf::from(format!("project/.usdhub/scenes/{}.usda", scene.id))
        );
    }

    #[test]
    fn authored_project_asset_paths_are_relative_and_contained() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let authoring = root.join("scenes/Lv1.usda");
        let target = root.join("models/Chair/model.usda");

        assert_eq!(
            authored_relative_project_asset_path(root, &authoring, &target).unwrap(),
            "../models/Chair/model.usda"
        );
        assert!(
            authored_relative_project_asset_path(root, &authoring, Path::new("/tmp/a.usda"))
                .is_err()
        );
    }

    #[test]
    fn local_state_roots_are_bootstrapped_without_object_or_session_children() {
        let directory = tempdir().unwrap();
        let layout = ProjectStorageLayout::new(directory.path());
        layout.ensure_local_state_roots().unwrap();

        assert!(layout.cache_dir().is_dir());
        assert!(layout.recovery_dir().is_dir());
        assert!(!layout.cache_objects_dir().exists());
        assert_eq!(fs::read_dir(layout.recovery_dir()).unwrap().count(), 0);
    }

    #[test]
    fn legacy_manifest_accessor_remains_separate_from_storage_v2() {
        let layout = ProjectStorageLayout::new("project");
        assert_eq!(
            layout.manifest_path(),
            PathBuf::from("project/project.json")
        );
        assert_eq!(
            layout.legacy_manifest_path(),
            PathBuf::from("project/.usdhub/project.json")
        );
    }
}
