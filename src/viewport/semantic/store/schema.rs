pub(crate) const SCHEMA_VERSION: i64 = 4;

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
    bim_enabled      INTEGER NOT NULL DEFAULT 0,
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

pub(super) async fn migrate(connection: &mut turso::Connection) -> anyhow::Result<()> {
    let current_version = {
        let mut rows = connection
            .query("SELECT COALESCE(MAX(version), 0) FROM schema_migrations", ())
            .await?;
        match rows.next().await? {
            Some(row) => row.get::<i64>(0)?,
            None => 0,
        }
    };
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
    let mut entity_rows = connection.query("PRAGMA table_info(entities)", ()).await?;
    let mut entity_columns = Vec::new();
    while let Some(row) = entity_rows.next().await? {
        entity_columns.push(row.get::<String>(1)?);
    }
    let has_bim_column = entity_columns.iter().any(|column| column == "bim_enabled");
    if !has_bim_column {
        connection
            .execute(
                "ALTER TABLE entities ADD COLUMN bim_enabled INTEGER",
                (),
            )
            .await?;
    }
    if current_version < SCHEMA_VERSION || !has_bim_column {
        let transaction = connection.transaction().await?;
        transaction.execute("DELETE FROM properties", ()).await?;
        transaction.execute("DELETE FROM entities", ()).await?;
        transaction.execute("DELETE FROM snapshots", ()).await?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO schema_migrations(version) VALUES (?1)",
                turso::params![SCHEMA_VERSION],
            )
            .await?;
        transaction.commit().await?;
    } else {
        connection
            .execute(
                "INSERT OR IGNORE INTO schema_migrations(version) VALUES (?1)",
                turso::params![SCHEMA_VERSION],
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{SCHEMA_VERSION, migrate};

    #[test]
    fn pre_bim_working_rows_are_invalidated_once_for_reingestion() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("migration test runtime builds");
        runtime.block_on(async {
            let database = turso::Builder::new_local(":memory:")
                .build()
                .await
                .expect("legacy working database builds");
            let mut connection = database.connect().expect("legacy working connection opens");
            connection
                .execute_batch(
                    r#"
                    CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY NOT NULL);
                    INSERT INTO schema_migrations(version) VALUES (1);
                    CREATE TABLE snapshots (snapshot_id TEXT PRIMARY KEY);
                    CREATE TABLE entities (
                        snapshot_id TEXT NOT NULL,
                        entity_key TEXT NOT NULL,
                        PRIMARY KEY (snapshot_id, entity_key)
                    );
                    CREATE TABLE properties (
                        snapshot_id TEXT NOT NULL,
                        entity_key TEXT NOT NULL,
                        name TEXT NOT NULL,
                        value_kind TEXT NOT NULL,
                        value_text TEXT,
                        value_integer INTEGER,
                        value_real REAL,
                        value_hash TEXT NOT NULL,
                        PRIMARY KEY (snapshot_id, entity_key, name)
                    );
                    INSERT INTO snapshots(snapshot_id) VALUES ('legacy');
                    INSERT INTO entities(snapshot_id, entity_key) VALUES ('legacy', 'bim-entity');
                    "#,
                )
                .await
                .expect("legacy working schema creates");

            migrate(&mut connection)
                .await
                .expect("legacy working migration succeeds");
            let mut rows = connection
                .query("SELECT COUNT(*) FROM entities", ())
                .await
                .expect("invalidated entity count query succeeds");
            let row = rows
                .next()
                .await
                .expect("invalidated entity count reads")
                .expect("invalidated entity count exists");
            assert_eq!(row.get::<i64>(0).expect("invalidated entity count decodes"), 0);

            connection
                .execute(
                    "INSERT INTO entities(snapshot_id, entity_key, bim_enabled)
                     VALUES ('reingested', 'bim-entity', 1)",
                    (),
                )
                .await
                .expect("reingested working entity inserts");
            migrate(&mut connection)
                .await
                .expect("idempotent working migration succeeds");
            let mut rows = connection
                .query(
                    "SELECT bim_enabled FROM entities WHERE snapshot_id = 'reingested'",
                    (),
                )
                .await
                .expect("reingested BIM query succeeds");
            let row = rows
                .next()
                .await
                .expect("reingested BIM row reads")
                .expect("reingested BIM row exists");
            assert_eq!(row.get::<i64>(0).expect("reingested BIM flag decodes"), 1);

            let mut rows = connection
                .query(
                    "SELECT version FROM schema_migrations ORDER BY version DESC LIMIT 1",
                    (),
                )
                .await
                .expect("working schema version query succeeds");
            let row = rows
                .next()
                .await
                .expect("working schema version reads")
                .expect("working schema version exists");
            assert_eq!(row.get::<i64>(0).expect("working schema version decodes"), SCHEMA_VERSION);
        });
    }
}
