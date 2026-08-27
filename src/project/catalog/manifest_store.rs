use std::{fs, fs::File, fs::OpenOptions, io::Write, path::Path};

use anyhow::{Context, Result};
use usd_project::ProjectManifestV1;
use uuid::Uuid;

const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";
const PROJECT_MANIFEST_FILE: &str = "project.json";

pub(crate) struct ManifestStore;

impl ManifestStore {
    pub(crate) fn write_manifest_atomic(
        project_root: &Path,
        manifest: &ProjectManifestV1,
    ) -> Result<()> {
        manifest.validate().context("validate Project manifest")?;
        let bytes = serde_json::to_vec_pretty(&manifest.canonicalized())
            .context("serialize canonical Project manifest")?;
        let directory = project_root.join(PROJECT_METADATA_DIRECTORY);
        fs::create_dir_all(&directory).context("create Project metadata directory")?;

        let temporary_path = directory.join(format!(".project.{}.tmp", Uuid::new_v4()));
        let final_path = directory.join(PROJECT_MANIFEST_FILE);
        write_bytes_atomic(&temporary_path, &final_path, &bytes)
    }
}

pub(crate) fn write_bytes_atomic(
    temporary_path: &Path,
    final_path: &Path,
    bytes: &[u8],
) -> Result<()> {
    let mut created = false;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temporary_path)
            .with_context(|| format!("create temporary file {}", temporary_path.display()))?;
        created = true;
        file.write_all(bytes)
            .with_context(|| format!("write temporary file {}", temporary_path.display()))?;
        file.sync_all()
            .with_context(|| format!("sync temporary file {}", temporary_path.display()))?;
        fs::rename(temporary_path, final_path).with_context(|| {
            format!(
                "publish temporary manifest {} as {}",
                temporary_path.display(),
                final_path.display()
            )
        })?;
        sync_parent_best_effort(final_path.parent());
        Ok(())
    })();

    if result.is_err() && created {
        let _ = fs::remove_file(temporary_path);
    }
    result
}

fn sync_parent_best_effort(parent: Option<&Path>) {
    let Some(parent) = parent else {
        return;
    };
    let Ok(directory) = File::open(parent) else {
        return;
    };
    let _ = directory.sync_all();
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use usd_project::{
        ModelManifestEntry, ModelSourceKind, ProjectId, ProjectManifestV1, ProjectRoot, SceneId,
        SceneManifestEntry, StorageKey,
    };

    use super::*;

    fn storage_key(value: &str) -> StorageKey {
        StorageKey::new(value).unwrap()
    }

    fn manifest() -> ProjectManifestV1 {
        let scene_a = SceneManifestEntry {
            id: SceneId::new_v4(),
            storage_key: storage_key("scene-a"),
        };
        let scene_b = SceneManifestEntry {
            id: SceneId::new_v4(),
            storage_key: storage_key("scene-b"),
        };
        ProjectManifestV1::new(
            ProjectId::new_v4(),
            "Project",
            ProjectRoot::Empty,
            vec![scene_b, scene_a],
            vec![ModelManifestEntry {
                id: usd_project::ModelId::new_v4(),
                source_kind: ModelSourceKind::Usd,
                storage_key: storage_key("model"),
            }],
        )
    }

    fn manifest_path(root: &Path) -> std::path::PathBuf {
        root.join(PROJECT_METADATA_DIRECTORY)
            .join(PROJECT_MANIFEST_FILE)
    }

    #[test]
    fn writes_canonical_manifest_to_same_metadata_directory() {
        let directory = tempdir().unwrap();
        let manifest = manifest();

        ManifestStore::write_manifest_atomic(directory.path(), &manifest).unwrap();

        let path = manifest_path(directory.path());
        let bytes = fs::read(&path).unwrap();
        let decoded: ProjectManifestV1 = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, manifest.canonicalized());
        assert!(
            fs::read_dir(path.parent().unwrap())
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().starts_with(".project."))
        );
    }

    #[test]
    fn invalid_manifest_does_not_replace_existing_file() {
        let directory = tempdir().unwrap();
        let valid = manifest();
        ManifestStore::write_manifest_atomic(directory.path(), &valid).unwrap();
        let path = manifest_path(directory.path());
        let before = fs::read(&path).unwrap();

        let mut invalid = valid;
        invalid.schema_version = 2;
        assert!(ManifestStore::write_manifest_atomic(directory.path(), &invalid).is_err());

        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(
            fs::read_dir(path.parent().unwrap())
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().starts_with(".project."))
        );
    }
}
