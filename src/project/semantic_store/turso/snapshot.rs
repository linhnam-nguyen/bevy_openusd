use anyhow::{Context, Result, bail};
use usd_model::{SemanticSnapshot, SnapshotId, SnapshotSource};

use super::TursoSemanticStore;
use super::entity::{insert_entity, insert_property};

impl TursoSemanticStore {
    pub(super) async fn snapshot_json_by_id(&self, id: &SnapshotId) -> Result<Option<String>> {
        let mut rows = self
            .connection
            .query(
                "SELECT snapshot_json FROM snapshots WHERE snapshot_id = ?1",
                turso::params![id.0.clone()],
            )
            .await
            .context("reading durable semantic snapshot payload")?;
        rows.next()
            .await
            .context("reading durable semantic snapshot payload row")?
            .map(|row| {
                row.get(0)
                    .context("decoding durable semantic snapshot payload")
            })
            .transpose()
    }

    pub(super) async fn snapshot_json_by_commit(&self, git_oid: &str) -> Result<Option<String>> {
        let mut rows = self
            .connection
            .query(
                "SELECT s.snapshot_json
                   FROM snapshot_aliases a
                   JOIN snapshots s ON s.snapshot_id = a.snapshot_id
                  WHERE a.git_oid = ?1
                  LIMIT 1",
                turso::params![git_oid.to_owned()],
            )
            .await
            .context("reading durable semantic commit alias")?;
        if let Some(row) = rows
            .next()
            .await
            .context("reading durable semantic commit alias row")?
        {
            return Ok(Some(
                row.get(0)
                    .context("decoding durable semantic commit alias")?,
            ));
        }

        let mut rows = self
            .connection
            .query(
                "SELECT snapshot_json FROM snapshots WHERE git_oid = ?1 LIMIT 1",
                turso::params![git_oid.to_owned()],
            )
            .await
            .context("reading durable semantic commit snapshot")?;
        rows.next()
            .await
            .context("reading durable semantic commit snapshot row")?
            .map(|row| {
                row.get(0)
                    .context("decoding durable semantic commit snapshot")
            })
            .transpose()
    }

    pub(super) async fn commit_alias_snapshot_id(&self, git_oid: &str) -> Result<Option<String>> {
        let mut rows = self
            .connection
            .query(
                "SELECT snapshot_id FROM snapshot_aliases WHERE git_oid = ?1",
                turso::params![git_oid.to_owned()],
            )
            .await
            .context("reading durable semantic commit alias id")?;
        rows.next()
            .await
            .context("reading durable semantic commit alias id row")?
            .map(|row| {
                row.get(0)
                    .context("decoding durable semantic commit alias id")
            })
            .transpose()
    }

    pub(super) async fn put_snapshot_impl(&mut self, snapshot: &SemanticSnapshot) -> Result<()> {
        let SnapshotSource::GitCommit { oid } = &snapshot.source else {
            bail!("durable semantic store accepts committed Git snapshots only")
        };

        if let Some(alias_snapshot_id) = self.commit_alias_snapshot_id(oid).await?
            && alias_snapshot_id != snapshot.snapshot_id.0
        {
            bail!("Git commit {oid} is already mapped to semantic snapshot {alias_snapshot_id}");
        }

        let existing = self.get_snapshot_impl(&snapshot.snapshot_id).await?;
        if let Some(existing) = existing.as_ref()
            && !same_snapshot_content(existing, snapshot)
        {
            bail!(
                "semantic snapshot {} is immutable and cannot be replaced",
                snapshot.snapshot_id.0
            );
        }

        let transaction = self
            .connection
            .transaction()
            .await
            .context("starting durable semantic snapshot transaction")?;

        if existing.is_none() {
            let payload =
                serde_json::to_string(snapshot).context("serializing durable semantic snapshot")?;
            transaction
                .execute(
                    "INSERT INTO snapshots
                        (snapshot_id, source_kind, git_oid, live_revision,
                         config_hash, created_at_unix_ms, snapshot_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    turso::params![
                        snapshot.snapshot_id.0.clone(),
                        "git_commit",
                        oid.clone(),
                        turso::Value::Null,
                        snapshot.config_hash.to_hex(),
                        unix_time_ms(),
                        payload,
                    ],
                )
                .await
                .context("inserting durable semantic snapshot")?;

            for entity in snapshot.entities.values() {
                insert_entity(&transaction, snapshot, entity).await?;
                for property in &entity.properties {
                    insert_property(
                        &transaction,
                        &snapshot.snapshot_id.0,
                        &entity.key.0,
                        &property.name,
                        &property.value,
                        property.measurement.as_ref(),
                    )
                    .await?;
                }
            }
        }

        transaction
            .execute(
                "INSERT OR IGNORE INTO snapshot_aliases(git_oid, snapshot_id)
                 VALUES (?1, ?2)",
                turso::params![oid.clone(), snapshot.snapshot_id.0.clone()],
            )
            .await
            .context("recording durable semantic commit alias")?;
        transaction
            .commit()
            .await
            .context("committing durable semantic snapshot")?;
        Ok(())
    }

    pub(super) async fn get_snapshot_impl(
        &self,
        id: &SnapshotId,
    ) -> Result<Option<SemanticSnapshot>> {
        let Some(payload) = self.snapshot_json_by_id(id).await? else {
            return Ok(None);
        };
        serde_json::from_str(&payload)
            .with_context(|| format!("deserializing durable semantic snapshot {}", id.0))
            .map(Some)
    }

    pub(super) async fn get_commit_snapshot_impl(
        &self,
        git_oid: &str,
    ) -> Result<Option<SemanticSnapshot>> {
        let Some(payload) = self.snapshot_json_by_commit(git_oid).await? else {
            return Ok(None);
        };
        let mut snapshot: SemanticSnapshot = serde_json::from_str(&payload)
            .with_context(|| format!("deserializing durable commit snapshot {git_oid}"))?;
        snapshot.source = SnapshotSource::GitCommit {
            oid: git_oid.to_owned(),
        };
        Ok(Some(snapshot))
    }
}

pub(super) fn same_snapshot_content(left: &SemanticSnapshot, right: &SemanticSnapshot) -> bool {
    left.snapshot_id == right.snapshot_id
        && left.config_hash == right.config_hash
        && left.entities == right.entities
}

pub(super) fn unix_time_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
