use std::path::Path;

use usd_bevy::{LiveRevision, LiveStage};

use super::{export_error, write_archive_with_root_source};

/// Write a Scene package using the exact root layer currently owned by a
/// LiveStage. Canonical dependency files are still resolved from the Project
/// closure, while the active root is serialized at the caller's revision.
pub(crate) fn write_live_stage_archive(
    project_root: &Path,
    manifest: &usd_project::ValidatedProjectManifest,
    root_scene: usd_project::SceneId,
    live_stage: &LiveStage,
    expected_live_revision: LiveRevision,
    destination: &Path,
) -> Result<(), project_protocol::ProjectWriteError> {
    if live_stage.current_revision() != expected_live_revision {
        return Err(export_error());
    }
    let temporary_root = tempfile::tempdir().map_err(|_| export_error())?;
    let root_source = temporary_root.path().join("active-root.usda");
    let root_source_string = root_source.to_string_lossy().into_owned();
    live_stage
        .stage
        .root_layer()
        .export(&root_source_string)
        .map_err(|_| export_error())?;
    if live_stage.current_revision() != expected_live_revision {
        return Err(export_error());
    }
    write_archive_with_root_source(
        project_root,
        manifest,
        root_scene,
        destination,
        Some(&root_source),
    )
}
