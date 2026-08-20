use anyhow::{Context, Result};
use usd_model::SemanticSnapshot;

use super::SemanticDatabase;
use super::row::{
    insert_entity, insert_property, optional_integer, optional_text, source_columns, unix_time_ms,
};

impl SemanticDatabase {
    pub(crate) async fn replace_snapshot(&mut self, snapshot: &SemanticSnapshot) -> Result<u32> {
        let transaction = self
            .connection
            .transaction()
            .await
            .context("starting semantic snapshot transaction")?;

        transaction
            .execute("DELETE FROM properties", ())
            .await
            .context("clearing semantic properties")?;
        transaction
            .execute("DELETE FROM entities", ())
            .await
            .context("clearing semantic entities")?;
        transaction
            .execute("DELETE FROM snapshots", ())
            .await
            .context("clearing semantic snapshots")?;

        let (source_kind, git_oid, live_revision) = source_columns(&snapshot.source);
        transaction
            .execute(
                "INSERT INTO snapshots
                    (snapshot_id, source_kind, git_oid, live_revision, config_hash, created_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                turso::params![
                    snapshot.snapshot_id.0.clone(),
                    source_kind,
                    optional_text(git_oid.as_deref()),
                    optional_integer(live_revision),
                    snapshot.config_hash.to_hex(),
                    unix_time_ms(),
                ],
            )
            .await
            .context("inserting semantic snapshot")?;

        for entity in snapshot.entities.values() {
            insert_entity(&transaction, snapshot, entity).await?;
            for property in &entity.properties {
                insert_property(
                    &transaction,
                    &snapshot.snapshot_id.0,
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
            .context("committing semantic snapshot")?;
        Ok(snapshot.entities.len() as u32)
    }

    pub(super) async fn latest_snapshot_id(&self) -> Result<Option<String>> {
        let mut rows = self
            .connection
            .query(
                "SELECT snapshot_id FROM snapshots ORDER BY created_at_unix_ms DESC LIMIT 1",
                (),
            )
            .await
            .context("reading latest semantic snapshot")?;
        rows.next()
            .await
            .context("reading latest semantic snapshot row")?
            .map(|row| row.get(0).context("decoding latest semantic snapshot"))
            .transpose()
    }
}
