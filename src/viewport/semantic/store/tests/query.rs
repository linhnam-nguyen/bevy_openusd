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

#[test]
fn display_name_filter_is_case_insensitive_path_exclusive_and_paginated() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");
    runtime.block_on(async {
        let usda = r#"#usda 1.0
def Xform "World"
{
    def Xform "Wall"
    {
    }
    def Xform "Door"
    {
    }
}
"#;
        let stage = usd_bevy::UsdSnippet::new(usda)
            .open_stage()
            .expect("stage opens");
        let extractor = SemanticExtractor::new(SemanticConfig::default());
        let mut snapshot = extractor
            .extract(
                &stage,
                SnapshotSource::Working {
                    session: "display-name-filter-test".to_owned(),
                    live_revision: 1,
                },
            )
            .expect("extract snapshot");

        let mut entities = snapshot.entities.values_mut();
        let wall = entities.next().expect("wall entity exists");
        wall.prim_path = "/Architecture/Level01/Wall_0042".to_owned();
        wall.semantic.display_name = Some("Exterior Wall".to_owned());
        let door = entities.next().expect("door entity exists");
        door.prim_path = "/Architecture/Level02/Door_0001".to_owned();
        door.semantic.display_name = Some("Exterior Door".to_owned());

        let mut database = SemanticDatabase::open().await.expect("database opens");
        database
            .replace_snapshot(&snapshot)
            .await
            .expect("replace snapshot");

        let wall_query = SemanticQuery {
            filters: vec![SemanticFilter::DisplayNameContains(
                "Exterior Wall".to_owned(),
            )],
            ..Default::default()
        };
        let wall_result = database.query(&wall_query).await.expect("query wall");
        assert_eq!(wall_result.total, 1);
        assert_eq!(wall_result.rows.len(), 1);
        assert_eq!(
            wall_result.rows[0].prim_path,
            "/Architecture/Level01/Wall_0042"
        );
        assert_eq!(
            wall_result.rows[0].display_name.as_deref(),
            Some("Exterior Wall")
        );
        assert!(!wall_result.has_more);

        let first_page = database
            .query(&SemanticQuery {
                filters: vec![SemanticFilter::DisplayNameContains("EXTERIOR".to_owned())],
                limit: 1,
                ..Default::default()
            })
            .await
            .expect("query first page");
        assert_eq!(first_page.total, 2);
        assert_eq!(first_page.rows.len(), 1);
        assert!(first_page.has_more);

        let second_page = database
            .query(&SemanticQuery {
                filters: vec![SemanticFilter::DisplayNameContains("exterior".to_owned())],
                offset: 1,
                limit: 1,
                ..Default::default()
            })
            .await
            .expect("query second page");
        assert_eq!(second_page.total, 2);
        assert_eq!(second_page.rows.len(), 1);
        assert!(!second_page.has_more);
        assert_ne!(first_page.rows[0].prim_path, second_page.rows[0].prim_path);

        for term in ["Wall_0042", "Level01", "Architecture"] {
            let result = database
                .query(&SemanticQuery {
                    filters: vec![SemanticFilter::DisplayNameContains(term.to_owned())],
                    ..Default::default()
                })
                .await
                .expect("query path-only term");
            assert_eq!(result.total, 0, "path term should not match: {term}");
            assert!(result.rows.is_empty());
        }

        let generic_result = database
            .query(&SemanticQuery {
                text: Some("Wall_0042".to_owned()),
                ..Default::default()
            })
            .await
            .expect("query generic text");
        assert_eq!(generic_result.total, 1);
        assert_eq!(
            generic_result.rows[0].prim_path,
            "/Architecture/Level01/Wall_0042"
        );
    });
}
