//! Turso schema, snapshot bulk loading, and parameterized semantic queries.

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use usd_model::{
    CanonicalValue, EntitySnapshot, IdentitySource, SemanticSnapshot, SnapshotId, SnapshotSource,
};

use super::SemanticIncrementalUpdate;
use super::query::{
    GroupField, SemanticFilter, SemanticGroup, SemanticQuery, SemanticQueryResult,
    SemanticQueryRow, SortField,
};

pub(crate) const SCHEMA_VERSION: i64 = 1;

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY NOT NULL
);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (1);

CREATE TABLE IF NOT EXISTS snapshots (
    snapshot_id        TEXT PRIMARY KEY,
    source_kind        TEXT NOT NULL,
    git_oid            TEXT,
    live_revision      INTEGER,
    config_hash        TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS entities (
    snapshot_id      TEXT NOT NULL,
    entity_key       TEXT NOT NULL,
    identity_source  TEXT NOT NULL,
    prim_path        TEXT NOT NULL,
    display_name     TEXT,
    category         TEXT,
    family           TEXT,
    type_name        TEXT,
    type_id          TEXT,
    transform_hash   TEXT NOT NULL,
    topology_hash    TEXT,
    shape_hash       TEXT,
    metadata_hash    TEXT NOT NULL,
    full_hash        TEXT NOT NULL,
    tx_mm            INTEGER,
    ty_mm            INTEGER,
    tz_mm            INTEGER,
    PRIMARY KEY (snapshot_id, entity_key)
);

CREATE INDEX IF NOT EXISTS idx_entities_path
ON entities(snapshot_id, prim_path);

CREATE INDEX IF NOT EXISTS idx_entities_category
ON entities(snapshot_id, category);

CREATE INDEX IF NOT EXISTS idx_entities_family
ON entities(snapshot_id, family);

CREATE INDEX IF NOT EXISTS idx_entities_type
ON entities(snapshot_id, type_name);

CREATE TABLE IF NOT EXISTS properties (
    snapshot_id   TEXT NOT NULL,
    entity_key    TEXT NOT NULL,
    name          TEXT NOT NULL,
    value_kind    TEXT NOT NULL,
    value_text    TEXT,
    value_integer INTEGER,
    value_real    REAL,
    value_hash    TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, entity_key, name)
);

CREATE INDEX IF NOT EXISTS idx_properties_name_text
ON properties(snapshot_id, name, value_text);

CREATE INDEX IF NOT EXISTS idx_properties_name_integer
ON properties(snapshot_id, name, value_integer);

CREATE INDEX IF NOT EXISTS idx_properties_name_real
ON properties(snapshot_id, name, value_real);
"#;

pub(crate) struct SemanticDatabase {
    _database: turso::Database,
    connection: turso::Connection,
}

impl SemanticDatabase {
    pub(crate) async fn open() -> Result<Self> {
        let database = turso::Builder::new_local(":memory:")
            .build()
            .await
            .context("opening in-memory Turso database")?;
        let connection = database
            .connect()
            .context("connecting to in-memory Turso database")?;
        connection
            .execute_batch(SCHEMA_SQL)
            .await
            .context("applying semantic Turso schema")?;
        Ok(Self {
            _database: database,
            connection,
        })
    }

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

    pub(crate) async fn query(&self, query: &SemanticQuery) -> Result<SemanticQueryResult> {
        let Some(snapshot_id) = self.latest_snapshot_id().await? else {
            return Ok(SemanticQueryResult::default());
        };

        let (where_sql, where_params) = build_where(query, &snapshot_id);
        let count_sql = format!("SELECT COUNT(*) FROM entities e {where_sql}");
        let count_row = self
            .connection
            .query(&count_sql, turso::params_from_iter(where_params.clone()))
            .await
            .context("counting semantic query results")?
            .next()
            .await
            .context("reading semantic query count")?
            .ok_or_else(|| anyhow!("semantic count query returned no row"))?;
        let total = count_row
            .get::<i64>(0)
            .context("decoding semantic query count")? as u32;

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
            .context("querying semantic entities")?;
        let mut result_rows = Vec::new();
        while let Some(row) = rows.next().await.context("reading semantic entity row")? {
            result_rows.push(SemanticQueryRow {
                entity_key: usd_model::EntityKey::from(row.get::<String>(0)?),
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

        let (_, group_params) = build_where(query, &snapshot_id);
        let groups = self.groups(query, &where_sql, &group_params).await?;
        Ok(SemanticQueryResult {
            total,
            has_more: query.offset.saturating_add(result_rows.len() as u32) < total,
            rows: result_rows,
            groups,
        })
    }

    async fn latest_snapshot_id(&self) -> Result<Option<String>> {
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

    async fn groups(
        &self,
        query: &SemanticQuery,
        where_sql: &str,
        where_params: &[turso::Value],
    ) -> Result<Vec<SemanticGroup>> {
        let mut groups = Vec::new();
        for field in &query.group_by {
            let column = group_column(*field);
            let sql = format!(
                "SELECT e.{column}, COUNT(*) FROM entities e {where_sql}
                 GROUP BY e.{column} ORDER BY COUNT(*) DESC, e.{column} ASC"
            );
            let mut rows = self
                .connection
                .query(&sql, turso::params_from_iter(where_params.to_vec()))
                .await
                .with_context(|| format!("grouping semantic entities by {field:?}"))?;
            while let Some(row) = rows.next().await.context("reading semantic group row")? {
                groups.push(SemanticGroup {
                    field: *field,
                    value: nullable_text(&row, 0)?,
                    count: row.get::<i64>(1)? as u32,
                });
            }
        }
        Ok(groups)
    }
}

async fn delete_entity_path(
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
        .with_context(|| format!("inserting semantic entity {}", entity.key.0))?;
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
        .with_context(|| format!("inserting semantic property {entity_key}.{name}"))?;
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

fn source_columns(source: &SnapshotSource) -> (&'static str, Option<String>, Option<i64>) {
    match source {
        SnapshotSource::Working { live_revision, .. } => {
            ("working", None, Some(*live_revision as i64))
        }
        SnapshotSource::GitCommit { oid } => ("git_commit", Some(oid.clone()), None),
    }
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

fn optional_integer(value: Option<i64>) -> turso::Value {
    value
        .map(turso::Value::Integer)
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
    use super::{SCHEMA_VERSION, SemanticDatabase};
    use usd_model::SnapshotSource;
    use usd_semantic::{SemanticConfig, SemanticExtractor};

    #[test]
    fn schema_migration_creates_version_one_tables() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");
        runtime.block_on(async {
            let database = SemanticDatabase::open().await.expect("database opens");
            let mut rows = database
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
        });
    }

    #[test]
    fn profiles_semantic_database_replace_snapshot_baseline() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");
        runtime.block_on(async {
            let mut usda = String::from("#usda 1.0\n\ndef Xform \"World\"\n{\n");
            for group in ["A", "B", "C"] {
                usda.push_str(&format!("    def Xform \"{group}\"\n    {{\n"));
                for i in 0..10 {
                    usda.push_str(&format!(
                        "        def Xform \"{group}{i}\"\n        {{\n        }}\n"
                    ));
                }
                usda.push_str("    }\n");
            }
            usda.push_str("}\n");

            let stage = usd_bevy::UsdSnippet::new(&usda)
                .open_stage()
                .expect("synthetic wide stage opens");
            let extractor = SemanticExtractor::new(SemanticConfig::default());
            let snapshot = extractor
                .extract(
                    &stage,
                    SnapshotSource::Working {
                        session: "turso-baseline-test".to_owned(),
                        live_revision: 1,
                    },
                )
                .expect("extract snapshot");

            assert_eq!(snapshot.entities.len(), 34);

            let mut database = SemanticDatabase::open().await.expect("database opens");

            // Initial replace_snapshot
            let initial_inserted = database
                .replace_snapshot(&snapshot)
                .await
                .expect("replace snapshot");
            assert_eq!(initial_inserted, 34);

            let mut rows = database
                .connection
                .query("SELECT COUNT(*) FROM entities", ())
                .await
                .expect("count entities");
            let row = rows.next().await.unwrap().unwrap();
            let rows_present_before: i64 = row.get(0).unwrap();
            assert_eq!(rows_present_before, 34);

            // Resync full replace_snapshot baseline (current behavior before subtree delta optimization)
            let start_resync = std::time::Instant::now();
            let resync_inserted = database
                .replace_snapshot(&snapshot)
                .await
                .expect("resync replace snapshot");
            let resync_db_elapsed = start_resync.elapsed();
            assert_eq!(resync_inserted, 34);

            let mut rows = database
                .connection
                .query("SELECT COUNT(*) FROM entities", ())
                .await
                .expect("count entities");
            let row = rows.next().await.unwrap().unwrap();
            let rows_present_after: i64 = row.get(0).unwrap();
            assert_eq!(rows_present_after, 34);

            println!(
                "M25 Baseline (Turso SemanticDatabase::replace_snapshot): rows_present_before={}, rows_removed=34, rows_inserted=34, rows_present_after={}, db_elapsed={:?}",
                rows_present_before,
                rows_present_after,
                resync_db_elapsed,
            );
        });
    }
}
