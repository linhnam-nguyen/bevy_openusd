use anyhow::Result;
use usd_model::EntityKey;

use super::super::{
    GroupField, SemanticFilter, SemanticQuery, SemanticResponse, SemanticSubmitError,
    SemanticWorkingStore,
};
use super::fixtures::{response, snapshot};
use std::time::Duration;

#[test]
fn full_snapshot_bulk_load_supports_type_and_property_queries() -> Result<()> {
    let store = SemanticWorkingStore::default();
    let snapshot = snapshot()?;
    let expected_entities = snapshot.entities.len() as u32;
    assert!(store.submit_snapshot("load-1", snapshot));
    assert!(matches!(
        response(&store),
        SemanticResponse::SnapshotLoaded {
            request_id,
            entity_count
        } if request_id == "load-1" && entity_count == expected_entities
    ));

    assert!(store.submit_query(
        "query-type",
        SemanticQuery {
            filters: vec![SemanticFilter::TypeEquals("Cube".to_owned())],
            ..Default::default()
        },
    ));
    let SemanticResponse::QueryResult { result, .. } = response(&store) else {
        panic!("expected query result")
    };
    assert_eq!(result.total, 1);
    assert_eq!(result.rows[0].entity_key, EntityKey::from("/World/Robot"));

    assert!(store.submit_query(
        "query-property",
        SemanticQuery {
            filters: vec![SemanticFilter::PropertyTextEquals {
                name: "userProperties:name".to_owned(),
                value: "cart_01".to_owned(),
            }],
            ..Default::default()
        },
    ));
    let SemanticResponse::QueryResult { result, .. } = response(&store) else {
        panic!("expected property query result")
    };
    assert_eq!(result.total, 1);
    assert_eq!(result.rows[0].prim_path, "/World/Robot");
    Ok(())
}

#[test]
fn schema_query_supports_grouping_and_pagination() -> Result<()> {
    let store = SemanticWorkingStore::default();
    assert!(store.submit_snapshot("load-2", snapshot()?));
    let _ = response(&store);
    assert!(store.submit_query(
        "query-group",
        SemanticQuery {
            group_by: vec![GroupField::TypeName],
            limit: 1,
            ..Default::default()
        },
    ));
    let SemanticResponse::QueryResult { result, .. } = response(&store) else {
        panic!("expected grouped query result")
    };
    assert!(result.total >= 2);
    assert_eq!(result.rows.len(), 1);
    assert!(!result.groups.is_empty());
    assert!(result.has_more);
    Ok(())
}

#[test]
fn benchmark_query_boundary_is_bounded_without_changing_normal_submission() {
    let normal_store = SemanticWorkingStore::default();
    assert!(
        normal_store
            .try_submit_query("normal-query", SemanticQuery::default())
            .is_ok()
    );

    let benchmark_store = SemanticWorkingStore::default();
    benchmark_store.configure_test_mode(Duration::from_secs(1), true);
    let mut queue_full = false;
    for index in 0..64 {
        if matches!(
            benchmark_store
                .try_submit_query(format!("benchmark-query-{index}"), SemanticQuery::default()),
            Err(SemanticSubmitError::QueueFull)
        ) {
            queue_full = true;
            break;
        }
    }

    assert!(
        queue_full,
        "benchmark query traffic must expose bounded backpressure"
    );
    let high_water = benchmark_store.query_queue_high_water();
    assert!(high_water > 0);
    assert!(
        high_water <= 8,
        "benchmark queue HWM exceeded its capacity: {high_water}"
    );
}
