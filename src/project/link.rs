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
use openusd::usd::{InitialLoadSet, Stage};
use serde::{Deserialize, Serialize};
use usd_project::{ProjectManifestV1, SceneId};
use uuid::Uuid;

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
    if binding_path(project_root, scene_id).is_file() {
        return match status(project_root, scene_id) {
            Ok(status) => Ok(Some(status)),
            Err(error) => {
                let linked = scene_wrapper_is_linked(scene_path)?;
                if linked {
                    Ok(Some(LinkedSourceStatus::SourceUnavailable))
                } else {
                    Err(error)
                }
            }
        };
    }
    let linked = scene_wrapper_is_linked(scene_path)?;
    Ok(linked.then_some(LinkedSourceStatus::SourceUnavailable))
}

/// Backfill the canonical linked-source marker for legacy wrappers while the
/// machine-local binding still proves that the Scene is linked. This is an
/// explicit project migration, never a read-path repair.
pub(crate) fn migrate_linked_source_provenance(
    project_root: &Path,
    manifest: &ProjectManifestV1,
) -> Result<()> {
    let layout = ProjectStorageLayout::new(project_root);
    for scene in &manifest.scenes {
        if !binding_path(project_root, scene.id).is_file() {
            continue;
        }
        let scene_path = layout.readable_scene_path(manifest, scene);
        if scene_path.is_file() {
            migrate_scene_wrapper_marker(&scene_path)?;
        }
    }
    Ok(())
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
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(path.as_ref())
        .context("open Scene wrapper for link status")?;
    Ok(crate::project::spatial::source_binding_is_linked(
        &stage.prim(SCENE_SOURCE_PRIM),
    )?)
}

fn migrate_scene_wrapper_marker(scene_path: &Path) -> Result<()> {
    let path = scene_path.to_string_lossy().into_owned();
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(&path)
        .context("open legacy Scene wrapper for link migration")?;
    let source_prim = stage.prim(SCENE_SOURCE_PRIM);
    ensure!(
        source_prim.is_defined()?,
        "legacy linked Scene wrapper source prim must be defined"
    );
    if crate::project::spatial::source_binding_marker(&source_prim)?.is_some() {
        return Ok(());
    }

    crate::project::spatial::author_source_binding_role(&source_prim, true)?;
    let temporary_path = scene_path.with_file_name(format!(
        ".{}.linked-source-migration-{}.tmp.usda",
        scene_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("scene"),
        Uuid::new_v4()
    ));
    let result = stage
        .root_layer()
        .export(temporary_path.to_string_lossy().as_ref())
        .context("export migrated linked Scene wrapper")
        .and_then(|_| {
            fs::rename(&temporary_path, scene_path).context("publish migrated linked Scene wrapper")
        });
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
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
#[path = "link_tests.rs"]
mod tests;
