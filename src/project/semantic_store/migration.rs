//! Turso schema migration entry point.

use std::collections::HashMap;

use anyhow::{Context, Result, ensure};
use usd_model::SemanticSnapshot;

use super::schema::SCHEMA_SQL;

pub(crate) async fn apply(connection: &mut turso::Connection) -> Result<()> {
    connection
        .execute_batch(SCHEMA_SQL)
        .await
        .context("applying durable semantic-store schema")?;
    let mut rows = connection
        .query("PRAGMA table_info(properties)", ())
        .await
        .context("reading durable semantic property schema")?;
    let mut columns = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .context("reading durable semantic property schema row")?
    {
        columns.push(
            row.get::<String>(1)
                .context("decoding durable semantic property column")?,
        );
    }
    for column in ["quantity_id", "canonical_unit_id", "source_unit_id"] {
        if !columns.iter().any(|existing| existing == column) {
            connection
                .execute(
                    &format!("ALTER TABLE properties ADD COLUMN {column} TEXT"),
                    (),
                )
                .await
                .with_context(|| format!("adding durable semantic property column {column}"))?;
        }
    }
    let mut entity_rows = connection
        .query("PRAGMA table_info(entities)", ())
        .await
        .context("reading durable semantic entity schema")?;
    let mut entity_columns = Vec::new();
    while let Some(row) = entity_rows
        .next()
        .await
        .context("reading durable semantic entity schema row")?
    {
        entity_columns.push(
            row.get::<String>(1)
                .context("decoding durable semantic entity column")?,
        );
    }
    if !entity_columns.iter().any(|column| column == "bim_enabled") {
        connection
            .execute(
                "ALTER TABLE entities ADD COLUMN bim_enabled INTEGER",
                (),
            )
            .await
            .context("adding durable semantic BIM flag")?;
    }
    backfill_bim_enabled(connection).await?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (4)",
            (),
        )
        .await
        .context("recording durable semantic schema version")?;
    Ok(())
}

/// Reconstruct the indexed BIM flag from the immutable semantic payload before
/// publishing the corrected schema version. This also repairs databases that
/// were already migrated by the earlier DEFAULT 0 implementation.
async fn backfill_bim_enabled(connection: &mut turso::Connection) -> Result<()> {
    let transaction = connection
        .transaction()
        .await
        .context("starting durable BIM backfill transaction")?;
    let truth = {
        let mut rows = transaction
            .query("SELECT snapshot_id, snapshot_json FROM snapshots", ())
            .await
            .context("reading durable semantic snapshots for BIM backfill")?;
        let mut truth = HashMap::new();
        while let Some(row) = rows
            .next()
            .await
            .context("reading durable semantic snapshot for BIM backfill")?
        {
            let snapshot_id = row
                .get::<String>(0)
                .context("decoding durable semantic snapshot id for BIM backfill")?;
            let payload = row
                .get::<String>(1)
                .context("decoding durable semantic snapshot JSON for BIM backfill")?;
            let snapshot: SemanticSnapshot = serde_json::from_str(&payload)
                .with_context(|| format!("decoding semantic snapshot {snapshot_id} for BIM backfill"))?;
            ensure!(
                snapshot.snapshot_id.0 == snapshot_id,
                "durable semantic snapshot payload id does not match its row"
            );
            for entity in snapshot.entities.values() {
                truth.insert(
                    (snapshot_id.clone(), entity.key.0.clone()),
                    entity.semantic.is_bim_entity(),
                );
            }
        }
        truth
    };
    let stored_keys = {
        let mut rows = transaction
            .query("SELECT snapshot_id, entity_key FROM entities", ())
            .await
            .context("reading durable entities for BIM backfill")?;
        let mut keys = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .context("reading durable entity for BIM backfill")?
        {
            keys.push((
                row.get::<String>(0)
                    .context("decoding durable entity snapshot id for BIM backfill")?,
                row.get::<String>(1)
                    .context("decoding durable entity key for BIM backfill")?,
            ));
        }
        keys
    };
    ensure!(
        stored_keys.len() == truth.len()
            && stored_keys.iter().all(|key| truth.contains_key(key)),
        "durable semantic entity rows do not match snapshot payloads for BIM backfill"
    );

    for ((snapshot_id, entity_key), enabled) in truth {
        transaction
            .execute(
                "UPDATE entities SET bim_enabled = ?1
                 WHERE snapshot_id = ?2 AND entity_key = ?3",
                turso::params![
                    turso::Value::Integer(if enabled { 1 } else { 0 }),
                    snapshot_id,
                    entity_key,
                ],
            )
            .await
            .context("backfilling durable semantic BIM flag")?;
    }
    transaction
        .commit()
        .await
        .context("committing durable BIM backfill transaction")?;
    Ok(())
}
