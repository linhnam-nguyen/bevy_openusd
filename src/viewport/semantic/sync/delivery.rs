use bevy::prelude::World;
use usd_bevy::LiveRevision;
use usd_model::SemanticSnapshot;

use crate::project::blob_store::FilesystemBlobStore;
use crate::project::ghost_cache::{attach_render_blobs, attach_render_blobs_for_entities};
use crate::project::recovery::RecoverySettings;
use crate::project::runtime_delivery::{build_runtime_delivery, into_delivery_parts};
use crate::viewport::api::RenderServerInterface;

use super::action::SemanticSyncAction;

pub(in crate::viewport::semantic) fn publish_runtime_delivery(
    world: &World,
    snapshot: &SemanticSnapshot,
) {
    let Some(interface) = world
        .get_resource::<RenderServerInterface>()
        .map(RenderServerInterface::shared)
    else {
        // The local/native viewer does not install the WebRTC delivery bus.
        return;
    };
    let Some(settings) = world.get_resource::<RecoverySettings>() else {
        interface.clear_runtime_delivery();
        return;
    };
    let store = match FilesystemBlobStore::new(
        settings
            .project_root
            .join(crate::project::blob_store::OBJECTS_DIRECTORY),
    ) {
        Ok(store) => store,
        Err(error) => {
            interface.clear_runtime_delivery();
            bevy::log::error!("[runtime-delivery] cannot create BlobStore: {error:#}");
            return;
        }
    };
    let bundle = match build_runtime_delivery(
        &store,
        snapshot,
        viewport_protocol::RuntimeProfile::NativeMedium,
    ) {
        Ok(bundle) => bundle,
        Err(error) => {
            interface.clear_runtime_delivery();
            bevy::log::warn!("[runtime-delivery] bundle publication skipped: {error:#}");
            return;
        }
    };
    let (manifest, blobs) = into_delivery_parts(bundle);
    if let Err(error) = interface.publish_runtime_delivery(manifest, blobs) {
        interface.clear_runtime_delivery();
        bevy::log::warn!("[runtime-delivery] bundle publication rejected: {error:?}");
    }
}

pub(in crate::viewport::semantic) fn attach_render_blobs_to_action(
    world: &mut World,
    action: &mut SemanticSyncAction,
    live_revision: LiveRevision,
    root_count: usize,
) {
    match action {
        SemanticSyncAction::Replace(snapshot) => attach_render_blobs(world, snapshot),
        SemanticSyncAction::Delta(update) => {
            let Some(map) = world.get_resource::<usd_bevy::PrimEntities>() else {
                bevy::log::warn!(
                    target: "ghost_cache",
                    resync_fallback_reason = "missing_prim_entities_index",
                    root_count = root_count,
                    live_revision = live_revision.0,
                    "[ghost-cache] PrimEntities resource missing from world; falling back to full attach_render_blobs"
                );
                attach_render_blobs(world, &mut update.snapshot);
                for upsert in &mut update.request.upserts {
                    if let Some(enriched) = update.snapshot.entities.get(&upsert.key) {
                        *upsert = enriched.clone();
                    }
                }
                return;
            };

            // Partial index corruption: PrimEntities exists, but an affected geometry prim has no index mapping
            let has_missing_mapping = update.request.upserts.iter().any(|entity| {
                entity
                    .geometry
                    .as_ref()
                    .is_some_and(|g| g.render_blob.is_none())
                    && map.entity(&entity.prim_path).is_none()
            });

            if has_missing_mapping {
                bevy::log::warn!(
                    target: "ghost_cache",
                    resync_fallback_reason = "partial_prim_entities_index_corruption",
                    root_count = root_count,
                    live_revision = live_revision.0,
                    "[ghost-cache] affected geometry entity missing from PrimEntities index; falling back to full attach_render_blobs"
                );
                attach_render_blobs(world, &mut update.snapshot);
                for upsert in &mut update.request.upserts {
                    if let Some(enriched) = update.snapshot.entities.get(&upsert.key) {
                        *upsert = enriched.clone();
                    }
                }
                return;
            }

            // Enrich only affected upserted semantic entities
            attach_render_blobs_for_entities(world, &mut update.request.upserts);
            // Copy enriched upserts back into update.snapshot.entities
            for upsert in &update.request.upserts {
                if let Some(entity) = update.snapshot.entities.get_mut(&upsert.key) {
                    entity.geometry = upsert.geometry.clone();
                }
            }
        }
    }
}
