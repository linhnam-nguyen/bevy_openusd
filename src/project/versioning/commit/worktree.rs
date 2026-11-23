//! Conditional publication of the one Scene covered by a live commit lease.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use project_protocol::{ProjectWriteError, ProjectWriteErrorCode};
use usd_project::{ProjectRoot, SceneId, ValidatedProjectManifest};

use crate::project::{
    catalog::manifest_store::ManifestStore, scene::authoring::scene_path_for_entry,
    storage::ProjectStorageLayout,
};

pub(super) struct LiveWorktreePublication {
    root: PathBuf,
    path: PathBuf,
    original: Vec<u8>,
    scene_id: SceneId,
}

impl LiveWorktreePublication {
    pub(super) fn prepare(
        project_root: &Path,
        head_tree: &Path,
        manifest: &ValidatedProjectManifest,
        scene_id: SceneId,
        snapshot: &[u8],
        opened_root_digest: Option<[u8; 32]>,
    ) -> Result<Self, ProjectWriteError> {
        let root = project_root.canonicalize().map_err(|_| conflict())?;
        let scene = manifest.scene(scene_id).ok_or_else(conflict)?;
        let path = scene_path_for_entry(
            &root,
            scene,
            manifest.raw().root == ProjectRoot::Scene(scene_id),
        );
        if ProjectStorageLayout::new(&root).readable_scene_path(manifest.raw(), scene) != path {
            return Err(conflict());
        }
        let original = read_scene(&root, &path)?;
        let relative = path.strip_prefix(&root).map_err(|_| conflict())?;
        if original != snapshot {
            let safe_source = match fs::read(head_tree.join(relative)) {
                Ok(head) => same_source_bytes(&head, &original),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    // An uncommitted Scene has no Git baseline. The immutable
                    // source captured by its live owner is the only fallback.
                    opened_root_digest.as_ref() == Some(blake3::hash(&original).as_bytes())
                }
                Err(_) => false,
            };
            if !safe_source {
                return Err(conflict());
            }
        }
        Ok(Self {
            root,
            path,
            original,
            scene_id,
        })
    }

    pub(super) fn verify_original(&self) -> Result<(), ProjectWriteError> {
        let manifest = ManifestStore::read_validated(&self.root).map_err(|_| conflict())?;
        let scene = manifest.scene(self.scene_id).ok_or_else(conflict)?;
        let current =
            ProjectStorageLayout::new(&self.root).readable_scene_path(manifest.raw(), scene);
        if current != self.path || read_scene(&self.root, &self.path)? != self.original {
            return Err(conflict());
        }
        Ok(())
    }

    pub(super) fn publish(&self, snapshot: &[u8]) -> Result<(), ProjectWriteError> {
        if self.original == snapshot {
            return self.verify_original();
        }
        let parent = self.path.parent().ok_or_else(conflict)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|_| conflict())?;
        temporary.write_all(snapshot).map_err(|_| conflict())?;
        temporary
            .as_file()
            .set_permissions(
                fs::metadata(&self.path)
                    .map_err(|_| conflict())?
                    .permissions(),
            )
            .map_err(|_| conflict())?;
        temporary.as_file().sync_all().map_err(|_| conflict())?;
        // The Project publisher and live authoring lease remain held. Stage the
        // replacement first so the final compare is adjacent to the atomic rename.
        self.verify_original()?;
        temporary.persist(&self.path).map_err(|_| conflict())?;
        if read_scene(&self.root, &self.path)? != snapshot {
            return Err(conflict());
        }
        Ok(())
    }
}

fn read_scene(root: &Path, path: &Path) -> Result<Vec<u8>, ProjectWriteError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| conflict())?;
    if !path.starts_with(root)
        || !metadata.is_file()
        || metadata.file_type().is_symlink()
        || path.canonicalize().map_err(|_| conflict())? != path
    {
        return Err(conflict());
    }
    fs::read(path).map_err(|_| conflict())
}

fn same_source_bytes(left: &[u8], right: &[u8]) -> bool {
    let lines = |bytes| {
        std::str::from_utf8(bytes)
            .ok()
            .map(|text| text.replace("\r\n", "\n"))
    };
    left == right || matches!((lines(left), lines(right)), (Some(a), Some(b)) if a == b)
}

fn conflict() -> ProjectWriteError {
    ProjectWriteError::Failed {
        code: ProjectWriteErrorCode::ConcurrentChange,
    }
}
