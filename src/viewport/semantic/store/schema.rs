pub(crate) const SCHEMA_VERSION: i64 = 2;

pub(super) const SCHEMA_SQL: &str = r#"
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

pub(super) async fn migrate(connection: &turso::Connection) -> anyhow::Result<()> {
    let mut rows = connection
        .query("PRAGMA table_info(properties)", ())
        .await?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next().await? {
        columns.push(row.get::<String>(1)?);
    }
    for column in ["quantity_id", "canonical_unit_id", "source_unit_id"] {
        if !columns.iter().any(|existing| existing == column) {
            connection
                .execute(
                    &format!("ALTER TABLE properties ADD COLUMN {column} TEXT"),
                    (),
                )
                .await?;
        }
    }
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (2)",
            (),
        )
        .await?;
    Ok(())
}
