use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use usd_model::{SemanticSnapshot, SnapshotId};

use super::SemanticDatabase;
use super::row::{
    insert_entity, insert_property, optional_integer, optional_text, source_columns, unix_time_ms,
};
use crate::viewport::semantic::SemanticIncrementalUpdate;

impl SemanticDatabase {
    pub(crate) async fn apply_delta(
        &mut self,
        update: &SemanticIncrementalUpdate,
    ) -> Result<(u32, u32)> {
        let previous_snapshot_id = self
            .latest_snapshot_id()
            .await?
            .ok_or_else(|| anyhow!("cannot apply semantic delta before a full snapshot load"))?;
        let snapshot_id = update.snapshot_id.0.as_str();
        let (source_kind, git_oid, live_revision) = source_columns(&update.source);
        let snapshot = SemanticSnapshot {
            snapshot_id: SnapshotId(update.snapshot_id.0.clone()),
            source: update.source.clone(),
            config_hash: update.config_hash,
            entities: HashMap::new(),
        };
        let transaction = self
            .connection
            .transaction()
            .await
            .context("starting semantic delta transaction")?;

        // The working store contains one logical snapshot. Move the indexed
        // rows to the new content-addressed id before applying row changes.
        transaction
            .execute(
                "UPDATE entities SET snapshot_id = ?1 WHERE snapshot_id = ?2",
                turso::params![snapshot_id.to_owned(), previous_snapshot_id.clone()],
            )
            .await
            .context("moving semantic entities to the new snapshot id")?;
        transaction
            .execute(
                "UPDATE properties SET snapshot_id = ?1 WHERE snapshot_id = ?2",
                turso::params![snapshot_id.to_owned(), previous_snapshot_id.clone()],
            )
            .await
            .context("moving semantic properties to the new snapshot id")?;
        transaction
            .execute(
                "UPDATE snapshots
                    SET snapshot_id = ?1, source_kind = ?2, git_oid = ?3,
                        live_revision = ?4, config_hash = ?5, created_at_unix_ms = ?6
                  WHERE snapshot_id = ?7",
                turso::params![
                    snapshot_id.to_owned(),
                    source_kind,
                    optional_text(git_oid.as_deref()),
                    optional_integer(live_revision),
                    update.config_hash.to_hex(),
                    unix_time_ms(),
                    previous_snapshot_id,
                ],
            )
            .await
            .context("updating semantic snapshot metadata")?;

        for path in &update.removed_paths {
            delete_entity_path(&transaction, snapshot_id, path).await?;
        }

        for entity in &update.upserts {
            // A prim's identity can change while its path stays stable. Remove
            // by path first so the entity primary key never leaves a stale row.
            delete_entity_path(&transaction, snapshot_id, &entity.prim_path).await?;
            insert_entity(&transaction, &snapshot, entity).await?;
            for property in &entity.properties {
                insert_property(
                    &transaction,
                    snapshot_id,
                    &entity.key.0,
                    &property.name,
                    &property.value,
                )
                .await?;
            }
        }

        transaction
            .commit()
            .await
            .context("committing semantic delta")?;
        Ok((
            update.upserts.len() as u32,
            update.removed_paths.len() as u32,
        ))
    }
}

pub(super) async fn delete_entity_path(
    transaction: &turso::transaction::Transaction<'_>,
    snapshot_id: &str,
    prim_path: &str,
) -> Result<()> {
    transaction
        .execute(
            "DELETE FROM properties
              WHERE snapshot_id = ?1
                AND entity_key IN (
                    SELECT entity_key FROM entities
                     WHERE snapshot_id = ?1 AND prim_path = ?2
                )",
            turso::params![snapshot_id.to_owned(), prim_path.to_owned()],
        )
        .await
        .with_context(|| format!("deleting semantic properties for {prim_path}"))?;
    transaction
        .execute(
            "DELETE FROM entities WHERE snapshot_id = ?1 AND prim_path = ?2",
            turso::params![snapshot_id.to_owned(), prim_path.to_owned()],
        )
        .await
        .with_context(|| format!("deleting semantic entity {prim_path}"))?;
    Ok(())
}
