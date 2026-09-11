use super::super::TursoSemanticStore;
use super::runtime;
use crate::project::semantic_store::SCHEMA_VERSION;

#[test]
fn schema_migration_creates_durable_snapshot_tables() {
    runtime().block_on(async {
        let store = TursoSemanticStore::open_memory()
            .await
            .expect("durable store opens");
        let mut rows = store
            .connection()
            .query(
                "SELECT version FROM schema_migrations ORDER BY version DESC LIMIT 1",
                (),
            )
            .await
            .expect("migration query succeeds");
        let row = rows
            .next()
            .await
            .expect("migration row reads")
            .expect("migration row exists");
        assert_eq!(row.get::<i64>(0).expect("version decodes"), SCHEMA_VERSION);

        let row = store
            .connection()
            .query(
                "SELECT COUNT(*) FROM pragma_table_info('properties')
                 WHERE name IN ('quantity_id', 'canonical_unit_id', 'source_unit_id')",
                (),
            )
            .await
            .expect("property schema query succeeds")
            .next()
            .await
            .expect("property schema row reads")
            .expect("property schema row exists");
        assert_eq!(row.get::<i64>(0).expect("property column count decodes"), 3);

        let row = store
            .connection()
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
fn legacy_bim_rows_are_backfilled_transactionally_and_idempotently() {
    runtime().block_on(async {
        let database = turso::Builder::new_local(":memory:")
            .build()
            .await
            .expect("legacy durable database builds");
        let mut connection = database.connect().expect("legacy durable connection opens");
        connection
            .execute_batch(
                r#"
                CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY NOT NULL);
                INSERT INTO schema_migrations(version) VALUES (2);
                CREATE TABLE snapshots (
                    snapshot_id TEXT PRIMARY KEY NOT NULL,
                    source_kind TEXT NOT NULL,
                    git_oid TEXT,
                    live_revision INTEGER,
                    config_hash TEXT NOT NULL,
                    created_at_unix_ms INTEGER NOT NULL,
                    snapshot_json TEXT NOT NULL
                );
                CREATE TABLE entities (
                    snapshot_id TEXT NOT NULL,
                    entity_key TEXT NOT NULL,
                    identity_source TEXT NOT NULL,
                    prim_path TEXT NOT NULL,
                    display_name TEXT,
                    category TEXT,
                    family TEXT,
                    type_name TEXT,
                    type_id TEXT,
                    transform_hash TEXT NOT NULL,
                    topology_hash TEXT,
                    shape_hash TEXT,
                    metadata_hash TEXT NOT NULL,
                    full_hash TEXT NOT NULL,
                    tx_mm INTEGER,
                    ty_mm INTEGER,
                    tz_mm INTEGER,
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
                "#,
            )
            .await
            .expect("legacy durable schema creates");

        let mut semantic_snapshot = super::snapshot("commit-a", "snapshot-a", "A", 1);
        semantic_snapshot
            .entities
            .values_mut()
            .next()
            .expect("legacy BIM entity exists")
            .semantic
            .bim
            .element_id = Some("element-42".to_owned());
        let payload = serde_json::to_string(&semantic_snapshot).expect("snapshot serializes");
        connection
            .execute(
                "INSERT INTO snapshots
                    (snapshot_id, source_kind, git_oid, live_revision, config_hash,
                     created_at_unix_ms, snapshot_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                turso::params![
                    "snapshot-a".to_owned(),
                    "git_commit".to_owned(),
                    "commit-a".to_owned(),
                    turso::Value::Null,
                    semantic_snapshot.config_hash.to_hex(),
                    1_i64,
                    payload,
                ],
            )
            .await
            .expect("legacy durable snapshot inserts");
        connection
            .execute(
                "INSERT INTO entities
                    (snapshot_id, entity_key, identity_source, prim_path, display_name,
                     category, family, type_name, type_id, transform_hash, topology_hash,
                     shape_hash, metadata_hash, full_hash, tx_mm, ty_mm, tz_mm)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                turso::params![
                    "snapshot-a".to_owned(),
                    "/World/Wall".to_owned(),
                    "prim_path".to_owned(),
                    "/World/Wall".to_owned(),
                    "Wall".to_owned(),
                    "Architecture".to_owned(),
                    "Wall".to_owned(),
                    "IfcWall".to_owned(),
                    "wall-type".to_owned(),
                    semantic_snapshot.entities.values().next().unwrap().transform.hash.to_hex(),
                    turso::Value::Null,
                    turso::Value::Null,
                    semantic_snapshot.entities.values().next().unwrap().metadata_hash.to_hex(),
                    semantic_snapshot.entities.values().next().unwrap().full_hash.to_hex(),
                    1_i64,
                    0_i64,
                    0_i64,
                ],
            )
            .await
            .expect("legacy durable entity inserts");

        crate::project::semantic_store::migration::apply(&mut connection)
            .await
            .expect("legacy durable BIM backfill succeeds");
        let mut rows = connection
            .query(
                "SELECT bim_enabled FROM entities WHERE snapshot_id = ?1 AND entity_key = ?2",
                turso::params!["snapshot-a".to_owned(), "/World/Wall".to_owned()],
            )
            .await
            .expect("backfilled BIM query succeeds");
        let row = rows
            .next()
            .await
            .expect("backfilled BIM row reads")
            .expect("backfilled BIM row exists");
        assert_eq!(row.get::<i64>(0).expect("backfilled BIM flag decodes"), 1);

        crate::project::semantic_store::migration::apply(&mut connection)
            .await
            .expect("idempotent durable BIM backfill succeeds");
        let mut rows = connection
            .query(
                "SELECT version FROM schema_migrations ORDER BY version DESC LIMIT 1",
                (),
            )
            .await
            .expect("idempotent migration version query succeeds");
        let row = rows
            .next()
            .await
            .expect("idempotent migration version reads")
            .expect("idempotent migration version exists");
        assert_eq!(row.get::<i64>(0).expect("schema version decodes"), SCHEMA_VERSION);
    });
}
