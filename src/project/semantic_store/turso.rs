//! Turso implementation of the durable semantic-store contract.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use usd_model::{
    CanonicalValue, EntityKey, EntitySnapshot, IdentitySource, SemanticSnapshot, SnapshotId,
    SnapshotSource,
};

use super::SemanticStore;
use super::migration;
use super::query::{
    GroupField, SemanticFilter, SemanticGroup, SemanticQuery, SemanticQueryResult,
    SemanticQueryRow, SortField,
};

pub(crate) struct TursoSemanticStore {
    _database: turso::Database,
    connection: turso::Connection,
}

impl TursoSemanticStore {
    pub(crate) async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path != Path::new(":memory:") {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("creating Turso parent directory {}", parent.display())
                })?;
            }
        }
        let path_string = path.to_string_lossy().into_owned();
        let database = turso::Builder::new_local(&path_string)
            .build()
            .await
            .with_context(|| format!("opening Turso semantic store at {}", path.display()))?;
        let connection = database
            .connect()
            .context("connecting to Turso semantic store")?;
        migration::apply(&connection).await?;
        Ok(Self {
            _database: database,
            connection,
        })
    }

    pub(crate) async fn open_memory() -> Result<Self> {
        Self::open(":memory:").await
    }

    async fn snapshot_json_by_id(&self, id: &SnapshotId) -> Result<Option<String>> {
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

    async fn snapshot_json_by_commit(&self, git_oid: &str) -> Result<Option<String>> {
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

    async fn commit_alias_snapshot_id(&self, git_oid: &str) -> Result<Option<String>> {
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
}

impl SemanticStore for TursoSemanticStore {
    async fn put_snapshot(&mut self, snapshot: &SemanticSnapshot) -> Result<()> {
        let SnapshotSource::GitCommit { oid } = &snapshot.source else {
            bail!("durable semantic store accepts committed Git snapshots only")
        };

        if let Some(alias_snapshot_id) = self.commit_alias_snapshot_id(oid).await?
            && alias_snapshot_id != snapshot.snapshot_id.0
        {
            bail!("Git commit {oid} is already mapped to semantic snapshot {alias_snapshot_id}");
        }

        let existing = self.get_snapshot(&snapshot.snapshot_id).await?;
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

    async fn get_snapshot(&self, id: &SnapshotId) -> Result<Option<SemanticSnapshot>> {
        let Some(payload) = self.snapshot_json_by_id(id).await? else {
            return Ok(None);
        };
        serde_json::from_str(&payload)
            .with_context(|| format!("deserializing durable semantic snapshot {}", id.0))
            .map(Some)
    }

    async fn get_entity(
        &self,
        snapshot: &SnapshotId,
        key: &EntityKey,
    ) -> Result<Option<EntitySnapshot>> {
        Ok(self
            .get_snapshot(snapshot)
            .await?
            .and_then(|snapshot| snapshot.entities.get(key).cloned()))
    }

    async fn get_commit_snapshot(&self, git_oid: &str) -> Result<Option<SemanticSnapshot>> {
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

    async fn query(
        &self,
        snapshot: &SnapshotId,
        query: &SemanticQuery,
    ) -> Result<SemanticQueryResult> {
        if self.get_snapshot(snapshot).await?.is_none() {
            return Ok(SemanticQueryResult::default());
        }

        let (where_sql, where_params) = build_where(query, &snapshot.0);
        let count_sql = format!("SELECT COUNT(*) FROM entities e {where_sql}");
        let count_row = self
            .connection
            .query(&count_sql, turso::params_from_iter(where_params.clone()))
            .await
            .context("counting durable semantic query results")?
            .next()
            .await
            .context("reading durable semantic query count")?
            .ok_or_else(|| anyhow!("durable semantic count query returned no row"))?;
        let total = count_row
            .get::<i64>(0)
            .context("decoding durable semantic query count")? as u32;

        let mut sql = format!(
            "SELECT e.entity_key, e.prim_path, e.display_name, e.category,
                    e.family, e.type_name, e.tx_mm, e.ty_mm, e.tz_mm
             FROM entities e {where_sql}"
        );
        append_order_by(&mut sql, query);
        sql.push_str(" LIMIT ? OFFSET ?");
        let mut params = where_params;
        params.push(turso::Value::Integer(limit(query.limit) as i64));
        params.push(turso::Value::Integer(query.offset as i64));

        let mut rows = self
            .connection
            .query(&sql, turso::params_from_iter(params))
            .await
            .context("querying durable semantic entities")?;
        let mut result_rows = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .context("reading durable semantic entity row")?
        {
            result_rows.push(SemanticQueryRow {
                entity_key: EntityKey::from(row.get::<String>(0)?),
                prim_path: row.get(1)?,
                display_name: nullable_text(&row, 2)?,
                category: nullable_text(&row, 3)?,
                family: nullable_text(&row, 4)?,
                type_name: nullable_text(&row, 5)?,
                translation_mm: [
                    nullable_integer(&row, 6)?.unwrap_or_default(),
                    nullable_integer(&row, 7)?.unwrap_or_default(),
                    nullable_integer(&row, 8)?.unwrap_or_default(),
                ],
            });
        }

        let (_, group_params) = build_where(query, &snapshot.0);
        let groups = groups(&self.connection, query, &where_sql, &group_params).await?;
        let row_count = result_rows.len() as u32;
        Ok(SemanticQueryResult {
            total,
            rows: result_rows,
            groups,
            has_more: query.offset.saturating_add(row_count) < total,
        })
    }
}

fn same_snapshot_content(left: &SemanticSnapshot, right: &SemanticSnapshot) -> bool {
    left.snapshot_id == right.snapshot_id
        && left.config_hash == right.config_hash
        && left.entities == right.entities
}

async fn groups(
    connection: &turso::Connection,
    query: &SemanticQuery,
    where_sql: &str,
    where_params: &[turso::Value],
) -> Result<Vec<SemanticGroup>> {
    let mut result = Vec::new();
    for field in &query.group_by {
        let column = group_column(*field);
        let sql = format!(
            "SELECT e.{column}, COUNT(*) FROM entities e {where_sql}
             GROUP BY e.{column} ORDER BY COUNT(*) DESC, e.{column} ASC"
        );
        let mut rows = connection
            .query(&sql, turso::params_from_iter(where_params.to_vec()))
            .await
            .with_context(|| format!("grouping durable semantic entities by {field:?}"))?;
        while let Some(row) = rows
            .next()
            .await
            .context("reading durable semantic group row")?
        {
            result.push(SemanticGroup {
                field: *field,
                value: nullable_text(&row, 0)?,
                count: row.get::<i64>(1)? as u32,
            });
        }
    }
    Ok(result)
}

async fn insert_entity(
    transaction: &turso::transaction::Transaction<'_>,
    snapshot: &SemanticSnapshot,
    entity: &EntitySnapshot,
) -> Result<()> {
    let geometry = entity.geometry.as_ref();
    transaction
        .execute(
            "INSERT INTO entities
                (snapshot_id, entity_key, identity_source, prim_path, display_name,
                 category, family, type_name, type_id, transform_hash, topology_hash,
                 shape_hash, metadata_hash, full_hash, tx_mm, ty_mm, tz_mm)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            turso::params![
                snapshot.snapshot_id.0.clone(),
                entity.key.0.clone(),
                identity_source_name(entity.identity_source),
                entity.prim_path.clone(),
                optional_text(entity.semantic.display_name.as_deref()),
                optional_text(entity.semantic.category.as_deref()),
                optional_text(entity.semantic.family.as_deref()),
                optional_text(entity.semantic.type_name.as_deref()),
                optional_text(entity.semantic.type_id.as_deref()),
                entity.transform.hash.to_hex(),
                geometry.map(|value| value.topology_hash.to_hex()),
                geometry.map(|value| value.shape_hash.to_hex()),
                entity.metadata_hash.to_hex(),
                entity.full_hash.to_hex(),
                turso::Value::Integer(entity.transform.translation_mm[0]),
                turso::Value::Integer(entity.transform.translation_mm[1]),
                turso::Value::Integer(entity.transform.translation_mm[2]),
            ],
        )
        .await
        .with_context(|| format!("inserting durable semantic entity {}", entity.key.0))?;
    Ok(())
}

async fn insert_property(
    transaction: &turso::transaction::Transaction<'_>,
    snapshot_id: &str,
    entity_key: &str,
    name: &str,
    value: &CanonicalValue,
) -> Result<()> {
    let (value_kind, value_text, value_integer, value_real, value_hash) = property_columns(value)?;
    transaction
        .execute(
            "INSERT INTO properties
                (snapshot_id, entity_key, name, value_kind, value_text,
                 value_integer, value_real, value_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            turso::params![
                snapshot_id.to_owned(),
                entity_key.to_owned(),
                name.to_owned(),
                value_kind,
                value_text,
                value_integer,
                value_real,
                value_hash,
            ],
        )
        .await
        .with_context(|| format!("inserting durable semantic property {entity_key}.{name}"))?;
    Ok(())
}

fn build_where(query: &SemanticQuery, snapshot_id: &str) -> (String, Vec<turso::Value>) {
    let mut clauses = vec!["e.snapshot_id = ?".to_owned()];
    let mut params = vec![turso::Value::Text(snapshot_id.to_owned())];
    if let Some(text) = query
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        clauses.push(
            "(LOWER(COALESCE(e.prim_path, '')) LIKE LOWER('%' || ? || '%')
             OR LOWER(COALESCE(e.display_name, '')) LIKE LOWER('%' || ? || '%')
             OR LOWER(COALESCE(e.category, '')) LIKE LOWER('%' || ? || '%')
             OR LOWER(COALESCE(e.family, '')) LIKE LOWER('%' || ? || '%')
             OR LOWER(COALESCE(e.type_name, '')) LIKE LOWER('%' || ? || '%')
             OR LOWER(COALESCE(e.type_id, '')) LIKE LOWER('%' || ? || '%'))"
                .to_owned(),
        );
        params.extend((0..6).map(|_| turso::Value::Text(text.to_owned())));
    }
    for filter in &query.filters {
        match filter {
            SemanticFilter::CategoryEquals(value) => {
                clauses.push("e.category = ?".to_owned());
                params.push(turso::Value::Text(value.clone()));
            }
            SemanticFilter::FamilyEquals(value) => {
                clauses.push("e.family = ?".to_owned());
                params.push(turso::Value::Text(value.clone()));
            }
            SemanticFilter::TypeEquals(value) => {
                clauses.push("e.type_name = ?".to_owned());
                params.push(turso::Value::Text(value.clone()));
            }
            SemanticFilter::PropertyTextEquals { name, value } => {
                clauses.push(
                    "EXISTS (SELECT 1 FROM properties p
                     WHERE p.snapshot_id = e.snapshot_id AND p.entity_key = e.entity_key
                       AND p.name = ? AND p.value_text = ?)"
                        .to_owned(),
                );
                params.push(turso::Value::Text(name.clone()));
                params.push(turso::Value::Text(value.clone()));
            }
            SemanticFilter::PropertyNumberRange { name, min, max } => {
                let mut clause = "EXISTS (SELECT 1 FROM properties p
                     WHERE p.snapshot_id = e.snapshot_id AND p.entity_key = e.entity_key
                       AND p.name = ?"
                    .to_owned();
                params.push(turso::Value::Text(name.clone()));
                if let Some(min) = min {
                    clause.push_str(" AND (p.value_real >= ? OR p.value_integer >= ?)");
                    params.push(turso::Value::Real(*min));
                    params.push(turso::Value::Real(*min));
                }
                if let Some(max) = max {
                    clause.push_str(" AND (p.value_real <= ? OR p.value_integer <= ?)");
                    params.push(turso::Value::Real(*max));
                    params.push(turso::Value::Real(*max));
                }
                clause.push(')');
                clauses.push(clause);
            }
        }
    }
    (format!("WHERE {}", clauses.join(" AND ")), params)
}

fn append_order_by(sql: &mut String, query: &SemanticQuery) {
    sql.push_str(" ORDER BY ");
    if query.sort.is_empty() {
        sql.push_str("e.prim_path ASC");
        return;
    }
    for (index, rule) in query.sort.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        sql.push_str(sort_column(rule.field));
        sql.push_str(if rule.descending { " DESC" } else { " ASC" });
    }
}

fn group_column(field: GroupField) -> &'static str {
    match field {
        GroupField::Category => "category",
        GroupField::Family => "family",
        GroupField::TypeName => "type_name",
    }
}

fn sort_column(field: SortField) -> &'static str {
    match field {
        SortField::DisplayName => "e.display_name",
        SortField::PrimPath => "e.prim_path",
        SortField::Category => "e.category",
        SortField::Family => "e.family",
        SortField::TypeName => "e.type_name",
        SortField::TranslationX => "e.tx_mm",
    }
}

fn limit(limit: u32) -> u32 {
    if limit == 0 { 100 } else { limit.min(1_000) }
}

fn identity_source_name(source: IdentitySource) -> &'static str {
    match source {
        IdentitySource::RevitUniqueId => "revit_unique_id",
        IdentitySource::IfcGuid => "ifc_guid",
        IdentitySource::ApplicationGuid => "application_guid",
        IdentitySource::AssetIdentifier => "asset_identifier",
        IdentitySource::PrimPath => "prim_path",
        IdentitySource::Synthetic => "synthetic",
    }
}

fn optional_text(value: Option<&str>) -> turso::Value {
    value
        .map(|value| turso::Value::Text(value.to_owned()))
        .unwrap_or(turso::Value::Null)
}

fn nullable_text(row: &turso::Row, index: usize) -> Result<Option<String>> {
    match row.get_value(index)? {
        turso::Value::Null => Ok(None),
        turso::Value::Text(value) => Ok(Some(value)),
        other => Err(anyhow!(
            "expected nullable text at column {index}, got {other:?}"
        )),
    }
}

fn nullable_integer(row: &turso::Row, index: usize) -> Result<Option<i64>> {
    match row.get_value(index)? {
        turso::Value::Null => Ok(None),
        turso::Value::Integer(value) => Ok(Some(value)),
        other => Err(anyhow!(
            "expected nullable integer at column {index}, got {other:?}"
        )),
    }
}

fn property_columns(
    value: &CanonicalValue,
) -> Result<(
    &'static str,
    Option<String>,
    Option<i64>,
    Option<f64>,
    String,
)> {
    let (kind, text, integer, real) = match value {
        CanonicalValue::Null => ("null", None, None, None),
        CanonicalValue::Bool(value) => ("bool", Some(value.to_string()), None, None),
        CanonicalValue::Integer(value) => ("integer", None, Some(*value), None),
        CanonicalValue::Real(value) => ("real", None, None, Some(*value)),
        CanonicalValue::Text(value) => ("text", Some(value.clone()), None, None),
        CanonicalValue::TextArray(values) => (
            "text_array",
            Some(serde_json::to_string(values)?),
            None,
            None,
        ),
        CanonicalValue::NumberArray(values) => (
            "number_array",
            Some(serde_json::to_string(values)?),
            None,
            None,
        ),
        CanonicalValue::Json(value) => ("json", Some(value.clone()), None, None),
    };
    let hash = blake3::hash(format!("{kind}:{text:?}:{integer:?}:{real:?}").as_bytes()).to_hex();
    Ok((kind, text, integer, real, hash.to_string()))
}

fn unix_time_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use usd_model::{
        CanonicalValue, EntityKey, EntitySnapshot, HashDigest, IdentitySource, SemanticInfo,
        SemanticProperty, SemanticSnapshot, SnapshotId, SnapshotSource, TransformSignature,
    };

    use super::TursoSemanticStore;
    use crate::project::semantic_store::SCHEMA_VERSION;
    use crate::project::semantic_store::{
        GroupField, SemanticFilter, SemanticQuery, SemanticStore, get_or_regenerate_commit_snapshot,
    };

    fn snapshot(oid: &str, snapshot_id: &str, comments: &str, seed: u8) -> SemanticSnapshot {
        let key = EntityKey::from("/World/Wall");
        let entity = EntitySnapshot {
            key: key.clone(),
            prim_path: "/World/Wall".to_owned(),
            identity_source: IdentitySource::PrimPath,
            semantic: SemanticInfo {
                category: Some("Architecture".to_owned()),
                family: Some("Wall".to_owned()),
                type_name: Some("IfcWall".to_owned()),
                type_id: Some("wall-type".to_owned()),
                display_name: Some("Wall".to_owned()),
            },
            transform: TransformSignature {
                translation_mm: [seed as i64, 0, 0],
                rotation_quantized: [0, 0, 0, 1],
                scale_quantized: [1, 1, 1],
                hash: HashDigest::new([seed; 32]),
            },
            geometry: None,
            properties: vec![SemanticProperty {
                name: "Comments".to_owned(),
                value: CanonicalValue::Text(comments.to_owned()),
            }],
            metadata_hash: HashDigest::new([seed.wrapping_add(1); 32]),
            full_hash: HashDigest::new([seed.wrapping_add(2); 32]),
        };
        let mut entities = HashMap::new();
        entities.insert(key, entity);
        SemanticSnapshot {
            snapshot_id: SnapshotId(snapshot_id.to_owned()),
            source: SnapshotSource::GitCommit {
                oid: oid.to_owned(),
            },
            config_hash: HashDigest::new([9; 32]),
            entities,
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
    }

    #[test]
    fn schema_migration_creates_durable_snapshot_tables() {
        runtime().block_on(async {
            let store = TursoSemanticStore::open_memory()
                .await
                .expect("durable store opens");
            let mut rows = store
                .connection
                .query("SELECT version FROM schema_migrations", ())
                .await
                .expect("migration query succeeds");
            let row = rows
                .next()
                .await
                .expect("migration row reads")
                .expect("migration row exists");
            assert_eq!(row.get::<i64>(0).expect("version decodes"), SCHEMA_VERSION);

            let row = store
                .connection
                .query(
                    "SELECT COUNT(*) FROM pragma_table_info('snapshots')
                     WHERE name = 'snapshot_json'",
                    (),
                )
                .await
                .expect("snapshot schema query succeeds")
                .next()
                .await
                .expect("snapshot schema row reads")
                .expect("snapshot schema row exists");
            assert_eq!(row.get::<i64>(0).expect("snapshot_json count decodes"), 1);
        });
    }

    #[test]
    fn put_get_entity_and_query_round_trip() {
        runtime().block_on(async {
            let mut store = TursoSemanticStore::open_memory()
                .await
                .expect("durable store opens");
            let expected = snapshot("commit-a", "snapshot-a", "A", 1);
            store
                .put_snapshot(&expected)
                .await
                .expect("snapshot persists");

            assert_eq!(
                store
                    .get_snapshot(&expected.snapshot_id)
                    .await
                    .expect("snapshot reads")
                    .expect("snapshot exists"),
                expected
            );
            let key = EntityKey::from("/World/Wall");
            assert_eq!(
                store
                    .get_entity(&expected.snapshot_id, &key)
                    .await
                    .expect("entity reads")
                    .expect("entity exists"),
                expected.entities[&key]
            );

            let category_result = store
                .query(
                    &expected.snapshot_id,
                    &SemanticQuery {
                        filters: vec![SemanticFilter::CategoryEquals("Architecture".to_owned())],
                        group_by: vec![GroupField::Category],
                        ..SemanticQuery::default()
                    },
                )
                .await
                .expect("category query succeeds");
            assert_eq!(category_result.total, 1);
            assert_eq!(category_result.rows[0].entity_key, key);
            assert_eq!(category_result.groups[0].count, 1);

            let property_result = store
                .query(
                    &expected.snapshot_id,
                    &SemanticQuery {
                        filters: vec![SemanticFilter::PropertyTextEquals {
                            name: "Comments".to_owned(),
                            value: "A".to_owned(),
                        }],
                        limit: 1,
                        ..SemanticQuery::default()
                    },
                )
                .await
                .expect("property query succeeds");
            assert_eq!(property_result.total, 1);
            assert!(!property_result.has_more);
        });
    }

    #[test]
    fn committed_snapshot_is_immutable_and_git_aliases_are_stable() {
        runtime().block_on(async {
            let mut store = TursoSemanticStore::open_memory()
                .await
                .expect("durable store opens");
            let first = snapshot("commit-a", "snapshot-a", "A", 1);
            store
                .put_snapshot(&first)
                .await
                .expect("first snapshot persists");
            store
                .put_snapshot(&first)
                .await
                .expect("idempotent snapshot write succeeds");

            let conflicting = snapshot("commit-a", "snapshot-a", "B", 2);
            assert!(store.put_snapshot(&conflicting).await.is_err());
            assert_eq!(
                store
                    .get_snapshot(&first.snapshot_id)
                    .await
                    .expect("snapshot reads")
                    .expect("snapshot remains present"),
                first
            );

            let same_content_other_commit = snapshot("commit-b", "snapshot-a", "A", 1);
            store
                .put_snapshot(&same_content_other_commit)
                .await
                .expect("same content can be aliased to another commit");
            let aliased = store
                .get_commit_snapshot("commit-b")
                .await
                .expect("commit alias reads")
                .expect("commit alias exists");
            assert_eq!(
                aliased.source,
                SnapshotSource::GitCommit {
                    oid: "commit-b".to_owned()
                }
            );
            assert_eq!(aliased.snapshot_id, first.snapshot_id);
        });
    }

    #[test]
    fn cache_miss_regeneration_is_persisted() {
        runtime().block_on(async {
            let mut store = TursoSemanticStore::open_memory()
                .await
                .expect("durable store opens");
            let calls = Arc::new(AtomicUsize::new(0));
            let first_calls = Arc::clone(&calls);
            let first = get_or_regenerate_commit_snapshot(&mut store, "commit-a", || async move {
                first_calls.fetch_add(1, Ordering::SeqCst);
                Ok(snapshot("commit-a", "snapshot-a", "A", 1))
            })
            .await
            .expect("cache miss regenerates");
            assert_eq!(first.snapshot_id, SnapshotId("snapshot-a".to_owned()));

            let second = get_or_regenerate_commit_snapshot(&mut store, "commit-a", || async {
                Err(anyhow::anyhow!("regenerator must not run on cache hit"))
            })
            .await
            .expect("cache hit reads persisted snapshot");
            assert_eq!(second, first);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn cached_commits_can_be_diffed_without_bevy() {
        runtime().block_on(async {
            let mut store = TursoSemanticStore::open_memory()
                .await
                .expect("durable store opens");
            let baseline = snapshot("commit-a", "snapshot-a", "A", 1);
            let current = snapshot("commit-b", "snapshot-b", "B", 2);
            store
                .put_snapshot(&baseline)
                .await
                .expect("baseline persists");
            store
                .put_snapshot(&current)
                .await
                .expect("current persists");

            let loaded_baseline = store
                .get_commit_snapshot("commit-a")
                .await
                .expect("baseline reads")
                .expect("baseline exists");
            let loaded_current = store
                .get_commit_snapshot("commit-b")
                .await
                .expect("current reads")
                .expect("current exists");
            let diff = usd_diff::compare(&loaded_baseline, &loaded_current);
            assert_eq!(diff.summary.changed, 1);
            assert_eq!(diff.summary.metadata, 1);
        });
    }
}
