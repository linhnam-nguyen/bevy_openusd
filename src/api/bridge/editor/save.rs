//! Authoritative local persistence for the working USD stage.

use std::{fs, path::Path};

use anyhow::Result;
use usd_bevy::LiveStage;
use viewport_protocol::EditorOperation;

use super::super::helpers::{emit_editor_completed, reject};
use super::super::state::EditorHistories;
use crate::cache::{
    ProjectCacheTarget, hydration::ActiveProjectCacheContext,
    warming::ProjectCacheWarmQueue,
};
use crate::project::{catalog::manifest_store::ManifestStore, storage::ProjectStorageLayout};
use crate::api::ViewportEventOutbox;
use crate::viewport::stage::SceneCacheOwnershipContext;

pub(super) fn save_stage_as(
    request_id: String,
    outbox: &mut ViewportEventOutbox,
    histories: &mut EditorHistories,
    stage: Option<&LiveStage>,
    filename: &str,
    active_project_cache: Option<&ActiveProjectCacheContext>,
    scene_cache_owner: Option<&SceneCacheOwnershipContext>,
    cache_warm: &ProjectCacheWarmQueue,
) {
    let Some(stage) = stage else {
        reject(outbox, request_id, "stage is not loaded".to_owned());
        return;
    };
    persist(
        request_id,
        outbox,
        histories,
        stage,
        filename,
        EditorOperation::SaveStageAs,
        active_project_cache,
        scene_cache_owner,
        cache_warm,
    );
}
pub(in crate::api::bridge) fn save_current_stage(
    request_id: String,
    outbox: &mut ViewportEventOutbox,
    histories: &mut EditorHistories,
    stage: Option<&LiveStage>,
    path: Option<&Path>,
    active_project_cache: Option<&ActiveProjectCacheContext>,
    scene_cache_owner: Option<&SceneCacheOwnershipContext>,
    cache_warm: &ProjectCacheWarmQueue,
) {
    let Some(stage) = stage else {
        reject(outbox, request_id, "stage is not loaded".to_owned());
        return;
    };
    let Some(path) = path else {
        reject(
            outbox,
            request_id,
            "current stage has no local save path".to_owned(),
        );
        return;
    };
    let filename = path.to_string_lossy();
    persist(
        request_id,
        outbox,
        histories,
        stage,
        &filename,
        EditorOperation::SaveStage,
        active_project_cache,
        scene_cache_owner,
        cache_warm,
    );
}

fn persist(
    request_id: String,
    outbox: &mut ViewportEventOutbox,
    histories: &mut EditorHistories,
    stage: &LiveStage,
    filename: &str,
    operation: EditorOperation,
    active_project_cache: Option<&ActiveProjectCacheContext>,
    scene_cache_owner: Option<&SceneCacheOwnershipContext>,
    cache_warm: &ProjectCacheWarmQueue,
) {
    if !stage.try_freeze_authoring() {
        reject(
            outbox,
            request_id,
            "the active document is already leased for publication".to_owned(),
        );
        return;
    }
    let written = persist_with_owned_timelines(
        stage, filename, &operation, active_project_cache, scene_cache_owner, cache_warm,
    );
    stage.unfreeze_authoring();
    if let Err(error) = written {
        reject(outbox, request_id, error.to_string());
        return;
    }
    histories.mark_saved();
    emit_editor_completed(outbox, request_id, operation, Vec::new(), histories);
}

fn persist_with_owned_timelines(
    stage: &LiveStage,
    filename: &str,
    operation: &EditorOperation,
    active_project_cache: Option<&ActiveProjectCacheContext>,
    scene_cache_owner: Option<&SceneCacheOwnershipContext>,
    cache_warm: &ProjectCacheWarmQueue,
) -> Result<()> {
    let destination = Path::new(filename);
    let (project_root, expected_scene_id) = if let Some(owner) = scene_cache_owner {
        (owner.project_root.clone(), Some(owner.scene_id.clone()))
    } else if let Some(context) = active_project_cache {
        (context.project_root.clone(), None)
    } else {
        return Ok(usd_bevy::authoring::save_stage_as(&stage.stage, filename)?);
    };
    let canonical_root = fs::canonicalize(&project_root)?;
    let lexical_destination = normalize_lexical_absolute(destination)?;
    let origin = identify_project_origin(&lexical_destination, &canonical_root)?;
    let canonical_destination = match fs::canonicalize(destination) {
        Ok(path) => path,
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && matches!(operation, EditorOperation::SaveStageAs) =>
        {
            // Resolve the existing directory even for a new external export.
            // Reject dangling symlinks and never recreate a missing managed layer.
            anyhow::ensure!(
                fs::symlink_metadata(destination)
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
                "unresolved save destination is not a new ordinary file"
            );
            let parent = destination
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            fs::canonicalize(parent)?.join(
                destination
                    .file_name()
                    .ok_or_else(|| anyhow::anyhow!("save destination has no filename"))?,
            )
        }
        Err(error) => return Err(error.into()),
    };
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let canonical_parent = fs::canonicalize(parent)?;
    let inside_resolved = canonical_destination.starts_with(&canonical_root);
    let inside_canonical_parent = canonical_parent.starts_with(&canonical_root);
    anyhow::ensure!(
        !(matches!(origin, SaveOrigin::ProjectOrigin) && !inside_resolved)
            && (!inside_canonical_parent || inside_resolved),
        "Project save destination escapes its canonical directory"
    );
    if !inside_resolved {
        anyhow::ensure!(
            matches!(operation, EditorOperation::SaveStageAs) || expected_scene_id.is_none(),
            "managed Scene Save must target its canonical Project layer"
        );
        return Ok(usd_bevy::authoring::save_stage_as(&stage.stage, filename)?);
    }
    let manifest = ManifestStore::read_validated(&project_root)?;
    let layout = ProjectStorageLayout::new(&project_root);
    let scene = manifest.scenes().iter().find(|scene| {
        let path = layout.readable_scene_path(manifest.raw(), scene);
        expected_scene_id.as_ref().is_none_or(|expected| expected == &scene.id)
            && fs::canonicalize(path).is_ok_and(|path| path == canonical_destination)
    }).ok_or_else(|| anyhow::anyhow!("unverified or missing managed Scene save destination: {}", destination.display()))?;
    let updated = crate::project::scene::timeline::save_scene_with_timelines_atomic(
        &stage.stage, &project_root, manifest.raw(), scene.id,
        &layout.readable_scene_path(manifest.raw(), scene),
    )?;
    let cache_targets = updated.into_iter().map(|id| ProjectCacheTarget::Scene { id: id.to_string() }).collect();
    if !cache_warm.enqueue_targets_for_mutation(&project_root, cache_targets).unwrap_or_else(|error| {
        log::warn!("canonical Scene saved but cache admission failed: {error:#}");
        false
    }) {
        log::warn!(
            "saved canonical Project Scene {} but derived-cache warm admission was unavailable",
            scene.id
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaveOrigin {
    ProjectOrigin,
    VerifiedExternalOrigin,
}

/// Determine whether any existing ancestor of the normalized user-supplied path
/// resolves via OS filesystem identity to `canonical_root` (or a subdirectory
/// thereof). Avoids casing heuristics so equivalent non-ASCII root spellings on
/// case-insensitive filesystems and distinct case-sensitive directories are both
/// classified accurately by the OS. Fails closed on non-`NotFound` errors.
fn identify_project_origin(
    lexical_destination: &Path,
    canonical_root: &Path,
) -> Result<SaveOrigin> {
    for ancestor in lexical_destination.ancestors() {
        match fs::canonicalize(ancestor) {
            Ok(canonical_ancestor) => {
                if canonical_ancestor == canonical_root
                    || canonical_ancestor.starts_with(canonical_root)
                {
                    return Ok(SaveOrigin::ProjectOrigin);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).map_err(|err| {
                    anyhow::anyhow!(
                        "unable to qualify save destination ancestor {}: {err}",
                        ancestor.display()
                    )
                });
            }
        }
    }
    Ok(SaveOrigin::VerifiedExternalOrigin)
}

/// Normalize a user-supplied path into an absolute lexical path (anchoring
/// relative paths to `current_dir` and folding `.`/`..`) without following
/// filesystem symlinks or Windows directory junctions.
fn normalize_lexical_absolute(path: &Path) -> Result<std::path::PathBuf> {
    use std::path::{Component, PathBuf};
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    let mut has_root = false;
    let mut normal_depth = 0usize;
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => {
                normalized.push(prefix.as_os_str());
            }
            Component::RootDir => {
                normalized.push(component.as_os_str());
                has_root = true;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if normal_depth > 0 {
                    normalized.pop();
                    normal_depth -= 1;
                } else if !has_root {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Normal(segment) => {
                normalized.push(segment);
                normal_depth += 1;
            }
        }
    }
    Ok(normalized)
}
