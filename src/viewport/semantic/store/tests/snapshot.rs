use usd_model::SnapshotSource;
use usd_semantic::{SemanticConfig, SemanticExtractor};

use super::super::{SCHEMA_VERSION, SemanticDatabase};

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
