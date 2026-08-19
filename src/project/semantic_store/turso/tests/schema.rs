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
