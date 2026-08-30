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
use openusd::usd::Stage;
use serde::{Deserialize, Serialize};
use usd_project::SceneId;

use super::storage::ProjectStorageLayout;

const BINDING_SCHEMA_VERSION: u32 = 2;
const SCENE_SOURCE_PRIM: &str = "/SceneRoot/Source";

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
    let binding = read_binding(project_root, scene_id)?;
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

/// Return the optional linked-source status used by the Project tree.
///
/// A linked Scene keeps its binding outside Git, so a cloned Project has no
/// binding file even though its canonical snapshot remains usable. The
/// canonical wrapper marker distinguishes that case from an ordinary import.
pub(crate) fn status_for_scene(
    project_root: &Path,
    scene_path: &Path,
    scene_id: SceneId,
) -> Result<Option<LinkedSourceStatus>> {
    let linked = scene_wrapper_is_linked(scene_path)?;
    if !binding_path(project_root, scene_id).is_file() {
        return Ok(linked.then_some(LinkedSourceStatus::SourceUnavailable));
    }
    match status(project_root, scene_id) {
        Ok(status) => Ok(Some(status)),
        Err(_error) if linked => Ok(Some(LinkedSourceStatus::SourceUnavailable)),
        Err(error) => Err(error),
    }
}

/// Resolve the authoritative source for a linked Scene. The caller supplies
/// only Project and Scene identities; source paths never come from the UI.
pub(crate) fn resolve_source(project_root: &Path, scene_id: SceneId) -> Result<PathBuf> {
    let binding = read_binding(project_root, scene_id)?;
    canonical_source(&binding.source_path)
}

fn read_binding(project_root: &Path, scene_id: SceneId) -> Result<LinkedSourceBinding> {
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
    Ok(binding)
}

fn scene_wrapper_is_linked(scene_path: &Path) -> Result<bool> {
    let path = scene_path.to_string_lossy();
    let stage = Stage::open(path.as_ref()).context("open Scene wrapper for link status")?;
    Ok(crate::project::spatial::source_binding_is_linked(
        &stage.prim(SCENE_SOURCE_PRIM),
    )?)
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
    use crate::project::scene::adoption_authoring::author_scene_wrapper_to_path;
    use crate::project::spatial::inspect_source;
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

    #[test]
    fn missing_binding_is_unavailable_only_for_linked_scene_wrappers() {
        let project = tempdir().unwrap();
        let source = project.path().join("source.usda");
        fs::write(
            &source,
            "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" {}\n",
        )
        .unwrap();
        let spatial = inspect_source(&source).unwrap();
        let linked_id = SceneId::new_v4();
        let linked_path = project.path().join("linked.usda");
        author_scene_wrapper_to_path(
            &linked_path,
            project.path(),
            &linked_path,
            linked_id,
            &source,
            "Assembly",
            "Linked",
            &spatial,
            true,
        )
        .unwrap();
        assert_eq!(
            status_for_scene(project.path(), &linked_path, linked_id).unwrap(),
            Some(LinkedSourceStatus::SourceUnavailable)
        );

        let imported_id = SceneId::new_v4();
        let imported_path = project.path().join("imported.usda");
        author_scene_wrapper_to_path(
            &imported_path,
            project.path(),
            &imported_path,
            imported_id,
            &source,
            "Assembly",
            "Imported",
            &spatial,
            false,
        )
        .unwrap();
        assert_eq!(
            status_for_scene(project.path(), &imported_path, imported_id).unwrap(),
            None
        );
    }
}
