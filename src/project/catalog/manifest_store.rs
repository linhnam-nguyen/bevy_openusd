use std::{fs, fs::File, fs::OpenOptions, io::Write, path::Path};

use anyhow::{Context, Result};
use usd_project::{ProjectManifestV1, ValidatedProjectManifest};
use uuid::Uuid;

use crate::project::storage::ProjectStorageLayout;

pub(crate) struct ManifestStore;

impl ManifestStore {
    pub(crate) fn write_manifest_atomic(
        project_root: &Path,
        manifest: &ProjectManifestV1,
    ) -> Result<()> {
        manifest.validate().context("validate Project manifest")?;
        let bytes = serde_json::to_vec_pretty(&manifest.canonicalized())
            .context("serialize canonical Project manifest")?;
        let directory = ProjectStorageLayout::new(project_root).root().to_owned();
        fs::create_dir_all(&directory).context("create Project metadata directory")?;

        let temporary_path = directory.join(format!(".project.{}.tmp", Uuid::new_v4()));
        let final_path = manifest_path(project_root);
        write_bytes_atomic(&temporary_path, &final_path, &bytes)
    }

    pub(crate) fn read_validated(project_root: &Path) -> Result<ValidatedProjectManifest> {
        let layout = ProjectStorageLayout::new(project_root);
        let canonical_manifest_present = layout.canonical_manifest_present();
        let path = layout.readable_manifest_path();
        let bytes =
            fs::read(&path).with_context(|| format!("read Project manifest {}", path.display()))?;
        let manifest: ProjectManifestV1 = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode Project manifest {}", path.display()))?;
        let migrated = manifest
            .clone()
            .migrate_legacy()
            .context("migrate Project manifest")?
            .canonicalized();
        migrated.validate().context("validate Project manifest")?;
        if canonical_manifest_present {
            crate::project::storage::recover_interrupted_migration(project_root, Some(&migrated))
                .context("recover interrupted Project storage migration")?;
            if migrated != manifest {
                Self::write_manifest_atomic(project_root, &migrated)
                    .context("persist migrated Project manifest")?;
            }
            return migrated
                .validate_and_index()
                .context("validate Project manifest");
        }
        if path == layout.legacy_manifest_path() && !canonical_manifest_present {
            crate::project::storage::ensure_legacy_migration_supported()
                .context("migrate legacy Project storage")?;
            crate::project::storage::recover_interrupted_migration(project_root, Some(&migrated))
                .context("recover interrupted Project storage migration")?;
            crate::project::storage::migrate_legacy_project(project_root, &migrated)
                .context("migrate legacy Project storage")?;
            return Self::read_validated(project_root);
        }
        migrated
            .validate_and_index()
            .context("validate Project manifest")
    }
}

pub(crate) fn manifest_path(project_root: &Path) -> std::path::PathBuf {
    ProjectStorageLayout::new(project_root).canonical_manifest_path()
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
#[path = "../../../tests/unit/project/catalog/manifest_store_tests.rs"]
mod tests;
