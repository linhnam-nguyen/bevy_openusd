//! Machine-local bindings for Project-managed linked Scene snapshots.
//!
//! The synchronized Scene wrapper and copied dependency closure remain
//! Git-tracked Project state. This sidecar is deliberately private local
//! state: it remembers where the source came from and the last opaque content
//! fingerprint without making an external path part of the Project model.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use usd_project::SceneId;

use super::storage::ProjectStorageLayout;

const BINDING_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct LinkedSourceBinding {
    pub(crate) schema_version: u32,
    pub(crate) scene_id: SceneId,
    pub(crate) source_path: PathBuf,
    pub(crate) source_fingerprint: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LinkedSourceStatus {
    InSync,
    SourceUnavailable,
    OutOfSync,
}

pub(crate) fn binding_path(project_root: &Path, scene_id: SceneId) -> PathBuf {
    ProjectStorageLayout::new(project_root)
        .links_dir()
        .join(format!("{scene_id}.json"))
}

pub(crate) fn prepare_binding(
    temporary_path: &Path,
    scene_id: SceneId,
    source: &Path,
) -> Result<()> {
    let source_path = canonical_source(source)?;
    let binding = LinkedSourceBinding {
        schema_version: BINDING_SCHEMA_VERSION,
        scene_id,
        source_fingerprint: source_fingerprint(&source_path)?,
        source_path,
    };
    let bytes = serde_json::to_vec_pretty(&binding).context("serialize linked source binding")?;
    if let Some(parent) = temporary_path.parent() {
        fs::create_dir_all(parent).context("create temporary linked source directory")?;
    }
    fs::write(temporary_path, bytes).context("write temporary linked source binding")
}

pub(crate) fn status(project_root: &Path, scene_id: SceneId) -> Result<LinkedSourceStatus> {
    let bytes =
        fs::read(binding_path(project_root, scene_id)).context("read linked source binding")?;
    let binding: LinkedSourceBinding =
        serde_json::from_slice(&bytes).context("decode linked source binding")?;
    ensure!(
        binding.schema_version == BINDING_SCHEMA_VERSION,
        "unsupported linked source binding schema"
    );
    ensure!(
        binding.scene_id == scene_id,
        "linked source binding identity mismatch"
    );
    if !binding.source_path.is_file() {
        return Ok(LinkedSourceStatus::SourceUnavailable);
    }
    Ok(
        if source_fingerprint(&binding.source_path)? == binding.source_fingerprint {
            LinkedSourceStatus::InSync
        } else {
            LinkedSourceStatus::OutOfSync
        },
    )
}

pub(crate) fn source_fingerprint(source: &Path) -> Result<String> {
    super::source_closure::source_closure_fingerprint(source)
}

fn canonical_source(source: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("read linked source metadata {}", source.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "linked source must be a regular non-symlink file"
    );
    fs::canonicalize(source)
        .with_context(|| format!("canonicalize linked source {}", source.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn status_distinguishes_sync_change_and_source_removal() {
        let project = tempdir().unwrap();
        let source = project.path().join("source.usda");
        fs::write(&source, b"#usda 1.0\n").unwrap();
        let temporary = project.path().join("binding.tmp");
        let scene_id = SceneId::new_v4();
        prepare_binding(&temporary, scene_id, &source).unwrap();
        fs::create_dir_all(ProjectStorageLayout::new(project.path()).links_dir()).unwrap();
        fs::rename(&temporary, binding_path(project.path(), scene_id)).unwrap();
        assert_eq!(
            status(project.path(), scene_id).unwrap(),
            LinkedSourceStatus::InSync
        );

        fs::write(&source, b"#usda 1.0\n# changed\n").unwrap();
        assert_eq!(
            status(project.path(), scene_id).unwrap(),
            LinkedSourceStatus::OutOfSync
        );

        fs::remove_file(&source).unwrap();
        assert_eq!(
            status(project.path(), scene_id).unwrap(),
            LinkedSourceStatus::SourceUnavailable
        );
    }

    #[test]
    fn status_detects_dependency_closure_change() {
        let project = tempdir().unwrap();
        let dependency = project.path().join("dependency.usda");
        fs::write(&dependency, "#usda 1.0\ndef Xform \"Asset\" {}\n").unwrap();
        let source = project.path().join("assembly.usda");
        fs::write(
            &source,
            "#usda 1.0\ndef Xform \"Assembly\" (references = @./dependency.usda@</Asset>) {}\n",
        )
        .unwrap();
        let scene_id = SceneId::new_v4();
        let binding_directory = tempdir().unwrap();
        let temporary = binding_directory.path().join("binding.tmp");
        prepare_binding(&temporary, scene_id, &source).unwrap();
        fs::create_dir_all(ProjectStorageLayout::new(project.path()).links_dir()).unwrap();
        fs::rename(&temporary, binding_path(project.path(), scene_id)).unwrap();
        assert_eq!(
            status(project.path(), scene_id).unwrap(),
            LinkedSourceStatus::InSync
        );

        fs::write(
            &dependency,
            "#usda 1.0\ndef Xform \"Asset\" { int changed = 1 }\n",
        )
        .unwrap();
        assert_eq!(
            status(project.path(), scene_id).unwrap(),
            LinkedSourceStatus::OutOfSync
        );
    }
}
