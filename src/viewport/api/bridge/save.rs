//! Authoritative local persistence for the working USD stage.

use std::{fs, path::Path};

use anyhow::Result;
use usd_bevy::LiveStage;
use viewport_protocol::EditorOperation;

use super::helpers::{emit_editor_completed, reject};
use super::state::EditorHistories;
use crate::project::{
    cache::ProjectCacheTarget,
    cache_hydration::ActiveProjectCacheContext,
    cache_warmer::ProjectCacheWarmQueue,
    catalog::manifest_store::ManifestStore,
    storage::ProjectStorageLayout,
};
use crate::viewport::api::ViewportEventOutbox;

pub(super) fn save_stage_as(
    request_id: String,
    outbox: &mut ViewportEventOutbox,
    histories: &mut EditorHistories,
    stage: Option<&LiveStage>,
    filename: &str,
    active_project_cache: Option<&ActiveProjectCacheContext>,
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
        cache_warm,
    );
}
pub(super) fn save_current_stage(
    request_id: String,
    outbox: &mut ViewportEventOutbox,
    histories: &mut EditorHistories,
    stage: Option<&LiveStage>,
    path: Option<&Path>,
    active_project_cache: Option<&ActiveProjectCacheContext>,
    cache_warm: &ProjectCacheWarmQueue,
) {
    let Some(stage) = stage else {
        reject(outbox, request_id, "stage is not loaded".to_owned());
        return;
    };
    let Some(path) = path else {
        reject(outbox, request_id, "current stage has no local save path".to_owned());
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
    cache_warm: &ProjectCacheWarmQueue,
) {
    if let Err(error) = usd_bevy::authoring::save_stage_as(&stage.stage, filename) {
        reject(outbox, request_id, error.to_string());
        return;
    }
    if let Err(error) =
        invalidate_owned_scene_after_save(Path::new(filename), active_project_cache, cache_warm)
    {
        reject(outbox, request_id, error.to_string());
        return;
    }
    histories.mark_saved();
    emit_editor_completed(outbox, request_id, operation, Vec::new(), histories);
}

fn invalidate_owned_scene_after_save(
    destination: &Path,
    active_project_cache: Option<&ActiveProjectCacheContext>,
    cache_warm: &ProjectCacheWarmQueue,
) -> Result<()> {
    let Some(context) = active_project_cache else {
        return Ok(());
    };
    let Ok(destination) = fs::canonicalize(destination) else {
        return Ok(());
    };
    let Ok(manifest) = ManifestStore::read_validated(&context.project_root) else {
        return Ok(());
    };
    let layout = ProjectStorageLayout::new(&context.project_root);
    let owner = manifest.scenes().iter().find(|scene| {
        let path = layout.readable_scene_path(manifest.raw(), scene);
        fs::canonicalize(path).is_ok_and(|path| path == destination)
    });
    let Some(scene) = owner else {
        return Ok(());
    };
    if !cache_warm.enqueue_targets_for_mutation(
        &context.project_root,
        vec![ProjectCacheTarget::Scene {
            id: scene.id.to_string(),
        }],
    )? {
        log::warn!(
            "saved canonical Project Scene {} but derived-cache warm admission was unavailable",
            scene.id
        );
    }
    Ok(())
}
