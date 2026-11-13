use openusd::sdf::Value;
use usd_model::SnapshotSource;
use usd_semantic::{SemanticConfig, SemanticExtractor};

use super::super::SemanticDatabase;

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

        let update = crate::viewport::semantic::SemanticIncrementalUpdate {
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
        assert_eq!(
            props.iter().filter(|(name, _)| name == "door_type").count(),
            1
        );
    });
}
