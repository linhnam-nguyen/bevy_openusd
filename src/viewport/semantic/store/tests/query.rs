use openusd::sdf::Value;
use usd_model::SnapshotSource;
use usd_semantic::{SemanticConfig, SemanticExtractor};

use super::super::SemanticDatabase;
use crate::viewport::semantic::query::{SemanticFilter, SemanticQuery};

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

        let update = crate::viewport::semantic::SemanticIncrementalUpdate {
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
