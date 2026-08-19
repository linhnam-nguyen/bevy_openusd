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
    use super::{SCHEMA_VERSION, SemanticDatabase, SemanticFilter, SemanticQuery};
    use openusd::sdf::Value;
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

    #[test]
    fn semantic_database_subtree_delta_updates_only_affected_rows_and_leaves_unaffected_untouched()
    {
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
            let source_1 = SnapshotSource::Working {
                session: "turso-delta-test".to_owned(),
                live_revision: 1,
            };
            let initial_snapshot = extractor
                .extract(&stage, source_1)
                .expect("extract initial snapshot");

            assert_eq!(initial_snapshot.entities.len(), 34);

            let mut database = SemanticDatabase::open().await.expect("database opens");

            // Initial replace_snapshot
            let initial_inserted = database
                .replace_snapshot(&initial_snapshot)
                .await
                .expect("initial replace snapshot");
            assert_eq!(initial_inserted, 34);

            // Read /World/B entity from SQLite before resync
            let mut rows_b = database
                .connection
                .query(
                    "SELECT entity_key, full_hash, transform_hash FROM entities WHERE prim_path = '/World/B'",
                    (),
                )
                .await
                .expect("query /World/B");
            let row_b = rows_b.next().await.unwrap().expect("/World/B row exists");
            let before_b_key: String = row_b.get(0).unwrap();
            let before_b_hash: String = row_b.get(1).unwrap();
            let before_b_tx_hash: String = row_b.get(2).unwrap();

            // Mutate stage under /World/A: remove /World/A/A9 and define /World/A/A_new
            stage.remove_prim("/World/A/A9").expect("remove A9");
            stage.define_prim("/World/A/A_new").expect("define A_new");

            // Extract updated /World/A subtree entities
            let mut upserts = Vec::new();
            let root_path = openusd::sdf::path("/World/A").unwrap();
            upserts.push(extractor.extract_entity(&stage, &root_path).unwrap());
            for i in 0..9 {
                let p = openusd::sdf::path(&format!("/World/A/A{i}")).unwrap();
                upserts.push(extractor.extract_entity(&stage, &p).unwrap());
            }
            let new_p = openusd::sdf::path("/World/A/A_new").unwrap();
            upserts.push(extractor.extract_entity(&stage, &new_p).unwrap());

            // 11 upserts for /World/A (/World/A + A0..A8 + A_new)
            assert_eq!(upserts.len(), 11);

            let removed_paths = vec!["/World/A/A9".to_owned()];

            let source_2 = SnapshotSource::Working {
                session: "turso-delta-test".to_owned(),
                live_revision: 2,
            };
            let updated_snapshot = extractor.extract(&stage, source_2.clone()).unwrap();

            let update = super::SemanticIncrementalUpdate {
                snapshot_id: updated_snapshot.snapshot_id.clone(),
                source: source_2,
                config_hash: updated_snapshot.config_hash,
                upserts,
                removed_paths,
            };

            // Apply subtree delta
            let start_delta = std::time::Instant::now();
            let (rows_upserted, rows_deleted) = database
                .apply_delta(&update)
                .await
                .expect("apply subtree delta");
            let delta_elapsed = start_delta.elapsed();

            // Deterministic counter assertions
            assert_eq!(rows_upserted, 11);
            assert_eq!(rows_deleted, 1);

            // Query total rows remaining
            let mut rows = database
                .connection
                .query("SELECT COUNT(*) FROM entities", ())
                .await
                .expect("count entities");
            let row = rows.next().await.unwrap().unwrap();
            let rows_remaining: i64 = row.get(0).unwrap();
            assert_eq!(rows_remaining, 34);

            // Assert /World/B entity in Turso remains byte-for-byte untouched
            let mut rows_b_after = database
                .connection
                .query(
                    "SELECT entity_key, full_hash, transform_hash FROM entities WHERE prim_path = '/World/B'",
                    (),
                )
                .await
                .expect("query /World/B after");
            let row_b_after = rows_b_after.next().await.unwrap().expect("/World/B row exists after");
            let after_b_key: String = row_b_after.get(0).unwrap();
            let after_b_hash: String = row_b_after.get(1).unwrap();
            let after_b_tx_hash: String = row_b_after.get(2).unwrap();

            assert_eq!(before_b_key, after_b_key);
            assert_eq!(before_b_hash, after_b_hash);
            assert_eq!(before_b_tx_hash, after_b_tx_hash);

            // Assert deleted row is gone and new row is present
            let mut rows_a9 = database
                .connection
                .query("SELECT COUNT(*) FROM entities WHERE prim_path = '/World/A/A9'", ())
                .await
                .unwrap();
            assert_eq!(rows_a9.next().await.unwrap().unwrap().get::<i64>(0).unwrap(), 0);

            let mut rows_anew = database
                .connection
                .query("SELECT COUNT(*) FROM entities WHERE prim_path = '/World/A/A_new'", ())
                .await
                .unwrap();
            assert_eq!(rows_anew.next().await.unwrap().unwrap().get::<i64>(0).unwrap(), 1);

            println!(
                "M25 Subtree Delta (Turso SemanticDatabase::apply_delta): rows_upserted={}, rows_deleted={}, rows_remaining={}, db_elapsed={:?}",
                rows_upserted,
                rows_deleted,
                rows_remaining,
                delta_elapsed,
            );
        });
    }

    #[test]
    fn test_query_after_subtree_delta_with_property_filters() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");
        runtime.block_on(async {
            let usda = r#"#usda 1.0
def Xform "World"
{
    def Xform "A"
    {
        def Xform "Child"
        {
            custom string material = "Steel"
        }
    }
    def Xform "B"
    {
        custom string material = "Concrete"
    }
}
"#;
            let stage = usd_bevy::UsdSnippet::new(usda)
                .open_stage()
                .expect("stage opens");
            let extractor = SemanticExtractor::new(SemanticConfig::default());
            let source_1 = SnapshotSource::Working {
                session: "query-delta-test".to_owned(),
                live_revision: 1,
            };
            let initial_snapshot = extractor
                .extract(&stage, source_1)
                .expect("extract initial");
            assert_eq!(initial_snapshot.entities.len(), 4);

            let mut database = SemanticDatabase::open().await.expect("database opens");
            database
                .replace_snapshot(&initial_snapshot)
                .await
                .expect("replace initial snapshot");

            // Mutate stage: remove Child, add New with custom property material = "Wood"
            stage.remove_prim("/World/A/Child").expect("remove Child");
            let new_prim = stage.define_prim("/World/A/New").expect("define New");
            new_prim
                .create_attribute("material", "string")
                .expect("create attribute")
                .set_custom(true)
                .expect("set custom")
                .set(Value::String("Wood".to_owned()))
                .expect("set attribute value");

            let source_2 = SnapshotSource::Working {
                session: "query-delta-test".to_owned(),
                live_revision: 2,
            };
            let updated_snapshot = extractor.extract(&stage, source_2.clone()).unwrap();

            let upsert_root = extractor
                .extract_entity(&stage, &openusd::sdf::path("/World/A").unwrap())
                .unwrap();
            let upsert_new = extractor
                .extract_entity(&stage, &openusd::sdf::path("/World/A/New").unwrap())
                .unwrap();

            let update = super::SemanticIncrementalUpdate {
                snapshot_id: updated_snapshot.snapshot_id.clone(),
                source: source_2,
                config_hash: updated_snapshot.config_hash,
                upserts: vec![upsert_root, upsert_new],
                removed_paths: vec!["/World/A/Child".to_owned()],
            };

            database.apply_delta(&update).await.expect("apply delta");

            // 1. General query
            let query = SemanticQuery::default();
            let result = database.query(&query).await.expect("query all");
            assert_eq!(result.total, 4); // /World, /World/A, /World/A/New, /World/B
            let paths: Vec<&str> = result.rows.iter().map(|r| r.prim_path.as_str()).collect();
            assert!(paths.contains(&"/World/A/New"));
            assert!(!paths.contains(&"/World/A/Child"));
            assert!(paths.contains(&"/World/B"));

            // 2. Property filter for material = "Concrete" on unaffected /World/B
            let query_b = SemanticQuery {
                filters: vec![SemanticFilter::PropertyTextEquals {
                    name: "material".to_owned(),
                    value: "Concrete".to_owned(),
                }],
                ..Default::default()
            };
            let result_b = database.query(&query_b).await.expect("query Concrete");
            assert_eq!(result_b.total, 1);
            assert_eq!(result_b.rows[0].prim_path, "/World/B");

            // 3. Property filter for material = "Wood" on newly added /World/A/New
            let query_new = SemanticQuery {
                filters: vec![SemanticFilter::PropertyTextEquals {
                    name: "material".to_owned(),
                    value: "Wood".to_owned(),
                }],
                ..Default::default()
            };
            let result_new = database.query(&query_new).await.expect("query Wood");
            assert_eq!(result_new.total, 1);
            assert_eq!(result_new.rows[0].prim_path, "/World/A/New");

            // 4. Property filter for material = "Steel" on deleted /World/A/Child
            let query_child = SemanticQuery {
                filters: vec![SemanticFilter::PropertyTextEquals {
                    name: "material".to_owned(),
                    value: "Steel".to_owned(),
                }],
                ..Default::default()
            };
            let result_child = database.query(&query_child).await.expect("query Steel");
            assert_eq!(result_child.total, 0);
            assert!(result_child.rows.is_empty());
        });
    }

    #[test]
    fn test_stable_key_path_move_after_delta() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");
        runtime.block_on(async {
            let usda = r#"#usda 1.0
def Xform "World"
{
    def Xform "A"
    {
        def Xform "Old"
        {
            string revit:uniqueId = "door-stable-001"
            custom string door_type = "Single"
        }
    }
    def Xform "B"
    {
    }
}
"#;
            let stage = usd_bevy::UsdSnippet::new(usda)
                .open_stage()
                .expect("stage opens");
            let mut config = SemanticConfig::default();
            config.identity.revit_unique_id_candidates = vec!["revit:uniqueId".to_string()];
            let extractor = SemanticExtractor::new(config.clone());
            let source_1 = SnapshotSource::Working {
                session: "stable-key-test".to_owned(),
                live_revision: 1,
            };
            let initial_snapshot = extractor
                .extract(&stage, source_1)
                .expect("extract initial");

            let mut database = SemanticDatabase::open().await.expect("database opens");
            database
                .replace_snapshot(&initial_snapshot)
                .await
                .expect("replace initial snapshot");

            // Move /World/A/Old to /World/A/New with same revit:uniqueId and updated property
            stage.remove_prim("/World/A/Old").expect("remove Old");
            let new_prim = stage.define_prim("/World/A/New").expect("define New");
            new_prim
                .create_attribute("revit:uniqueId", "string")
                .expect("create revit:uniqueId")
                .set(Value::String("door-stable-001".to_owned()))
                .expect("set revit:uniqueId");
            new_prim
                .create_attribute("door_type", "string")
                .expect("create door_type")
                .set_custom(true)
                .expect("set custom")
                .set(Value::String("Double".to_owned()))
                .expect("set door_type");

            let source_2 = SnapshotSource::Working {
                session: "stable-key-test".to_owned(),
                live_revision: 2,
            };
            let updated_snapshot = extractor.extract(&stage, source_2.clone()).unwrap();

            let upsert_new = extractor
                .extract_entity(&stage, &openusd::sdf::path("/World/A/New").unwrap())
                .unwrap();
            assert_eq!(upsert_new.key.as_str(), "revit:door-stable-001");

            let update = super::SemanticIncrementalUpdate {
                snapshot_id: updated_snapshot.snapshot_id.clone(),
                source: source_2,
                config_hash: updated_snapshot.config_hash,
                upserts: vec![upsert_new],
                removed_paths: vec!["/World/A/Old".to_owned()],
            };

            database.apply_delta(&update).await.expect("apply delta");

            // 1. Old path count = 0
            let mut rows_old = database
                .connection
                .query(
                    "SELECT COUNT(*) FROM entities WHERE prim_path = '/World/A/Old'",
                    (),
                )
                .await
                .unwrap();
            let old_count: i64 = rows_old.next().await.unwrap().unwrap().get(0).unwrap();
            assert_eq!(old_count, 0);

            // 2. New path count = 1
            let mut rows_new = database
                .connection
                .query(
                    "SELECT COUNT(*) FROM entities WHERE prim_path = '/World/A/New'",
                    (),
                )
                .await
                .unwrap();
            let new_count: i64 = rows_new.next().await.unwrap().unwrap().get(0).unwrap();
            assert_eq!(new_count, 1);

            // 3. EntityKey row count = 1
            let mut rows_key = database
                .connection
                .query(
                    "SELECT COUNT(*) FROM entities WHERE entity_key = 'revit:door-stable-001'",
                    (),
                )
                .await
                .unwrap();
            let key_count: i64 = rows_key.next().await.unwrap().unwrap().get(0).unwrap();
            assert_eq!(key_count, 1);

            // 4. Properties for EntityKey K are not duplicated / stale
            let mut rows_props = database
                .connection
                .query(
                    "SELECT name, value_text FROM properties WHERE entity_key = 'revit:door-stable-001' ORDER BY name ASC",
                    (),
                )
                .await
                .unwrap();
            let mut props = Vec::new();
            while let Some(row) = rows_props.next().await.unwrap() {
                let name: String = row.get(0).unwrap();
                let val: Option<String> = row.get(1).unwrap();
                props.push((name, val));
            }
            // Exactly 2 properties: door_type updated to "Double" (no stale "Single"), and revit:uniqueId
            assert_eq!(props.len(), 2);
            assert_eq!(props[0].0, "door_type");
            assert_eq!(props[0].1.as_deref(), Some("Double"));
            assert_eq!(props[1].0, "revit:uniqueId");
            assert_eq!(props[1].1.as_deref(), Some("door-stable-001"));
            assert_eq!(props.iter().filter(|(name, _)| name == "door_type").count(), 1);
        });
    }
}
