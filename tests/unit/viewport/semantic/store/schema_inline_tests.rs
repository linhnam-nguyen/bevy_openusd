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
        assert_eq!(
            row.get::<i64>(0).expect("invalidated entity count decodes"),
            0
        );

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
        assert_eq!(
            row.get::<i64>(0).expect("working schema version decodes"),
            SCHEMA_VERSION
        );
    });
}
