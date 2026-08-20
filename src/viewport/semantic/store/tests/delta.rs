use usd_model::SnapshotSource;
use usd_semantic::{SemanticConfig, SemanticExtractor};

use super::super::SemanticDatabase;

#[test]
fn semantic_database_subtree_delta_updates_only_affected_rows_and_leaves_unaffected_untouched() {
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

        let update = crate::viewport::semantic::SemanticIncrementalUpdate {
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
        let row_b_after = rows_b_after
            .next()
            .await
            .unwrap()
            .expect("/World/B row exists after");
        let after_b_key: String = row_b_after.get(0).unwrap();
        let after_b_hash: String = row_b_after.get(1).unwrap();
        let after_b_tx_hash: String = row_b_after.get(2).unwrap();

        assert_eq!(before_b_key, after_b_key);
        assert_eq!(before_b_hash, after_b_hash);
        assert_eq!(before_b_tx_hash, after_b_tx_hash);

        // Assert deleted row is gone and new row is present
        let mut rows_a9 = database
            .connection
            .query(
                "SELECT COUNT(*) FROM entities WHERE prim_path = '/World/A/A9'",
                (),
            )
            .await
            .unwrap();
        assert_eq!(
            rows_a9.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            0
        );

        let mut rows_anew = database
            .connection
            .query(
                "SELECT COUNT(*) FROM entities WHERE prim_path = '/World/A/A_new'",
                (),
            )
            .await
            .unwrap();
        assert_eq!(
            rows_anew
                .next()
                .await
                .unwrap()
                .unwrap()
                .get::<i64>(0)
                .unwrap(),
            1
        );

        println!(
            "M25 Subtree Delta (Turso SemanticDatabase::apply_delta): rows_upserted={}, rows_deleted={}, rows_remaining={}, db_elapsed={:?}",
            rows_upserted,
            rows_deleted,
            rows_remaining,
            delta_elapsed,
        );
    });
}
