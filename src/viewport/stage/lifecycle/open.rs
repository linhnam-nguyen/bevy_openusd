use bevy::prelude::*;
use usd_bevy::LiveStage;

use super::{StageHandle, StageInfo, StagePresentationContext, project_activation};
use crate::viewport::stage::SceneCacheOwnershipContext;

pub(super) fn open_stage(world: &mut World, path: std::path::PathBuf) {
    let active = world.get_non_send::<LiveStage>();
    if active.is_some_and(LiveStage::is_authoring_frozen) {
        warn!("cannot replace the active USD stage while authoring is frozen");
        return;
    }
    let has_active_stage = active.is_some();
    let same_document = world
        .get_resource::<StageHandle>()
        .is_some_and(|handle| handle.path == path);
    let (presentation, generation, owner) = if same_document {
        (
            world
                .get_resource::<StagePresentationContext>()
                .cloned()
                .unwrap_or_default(),
            world
                .get_resource::<StageInfo>()
                .map_or(0, |info| info.activation_generation),
            world.get_resource::<SceneCacheOwnershipContext>().cloned(),
        )
    } else {
        (StagePresentationContext::default(), 0, None)
    };
    let result = LiveStage::open_document(&path)
        .map_err(|error| format!("failed to open {}: {error:#}", path.display()))
        .and_then(|stage| {
            if let Some(active) = world.get_non_send::<LiveStage>()
                && same_document
            {
                reject_loss_of_edits(&active.stage, &stage.stage)
                    .map_err(|error| error.to_string())?;
            }
            project_activation::activate_open_stage_with_cache_context_for_generation(
                world,
                path.clone(),
                stage,
                None,
                None,
                owner.as_ref().map(|owner| owner.project_root.clone()),
                owner.as_ref().map(|owner| owner.scene_id),
                None,
                generation,
                presentation,
                project_activation::StageInstallMode::Fresh,
            )?;
            if let Some(owner) = owner {
                world.insert_resource(owner);
            }
            Ok(())
        });
    if let Err(message) = result {
        error!("{message}");
        if !has_active_stage {
            world.insert_resource(StageHandle {
                path,
                error: Some(message),
            });
        }
    }
}

fn reject_loss_of_edits(
    active: &openusd::usd::Stage,
    candidate: &openusd::usd::Stage,
) -> anyhow::Result<()> {
    for identifier in active.layer_identifiers() {
        let Some(layer) = active.layer(&identifier)? else {
            continue;
        };
        if !layer.try_borrow()?.is_dirty() {
            continue;
        }
        // Export-based saves can leave the native dirty bit set. Compare the
        // authored layer data before deciding that replacement would lose edits.
        if let Some(reloaded) = candidate.layer(&identifier)?
            && layer.export_to_string()? == reloaded.export_to_string()?
        {
            continue;
        }
        anyhow::bail!("cannot reload USD while layer {identifier} has unsaved edits");
    }
    Ok(())
}
