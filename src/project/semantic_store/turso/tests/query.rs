use usd_model::EntityKey;

use super::super::TursoSemanticStore;
use super::{runtime, snapshot};
use crate::project::semantic_store::{GroupField, SemanticFilter, SemanticQuery, SemanticStore};

#[test]
fn put_get_entity_and_query_round_trip() {
    runtime().block_on(async {
        let mut store = TursoSemanticStore::open_memory()
            .await
            .expect("durable store opens");
        let expected = snapshot("commit-a", "snapshot-a", "A", 1);
        store
            .put_snapshot(&expected)
            .await
            .expect("snapshot persists");

        assert_eq!(
            store
                .get_snapshot(&expected.snapshot_id)
                .await
                .expect("snapshot reads")
                .expect("snapshot exists"),
            expected
        );
        let key = EntityKey::from("/World/Wall");
        assert_eq!(
            store
                .get_entity(&expected.snapshot_id, &key)
                .await
                .expect("entity reads")
                .expect("entity exists"),
            expected.entities[&key]
        );

        let category_result = store
            .query(
                &expected.snapshot_id,
                &SemanticQuery {
                    filters: vec![SemanticFilter::CategoryEquals("Architecture".to_owned())],
                    group_by: vec![GroupField::Category],
                    ..SemanticQuery::default()
                },
            )
            .await
            .expect("category query succeeds");
        assert_eq!(category_result.total, 1);
        assert_eq!(category_result.rows[0].entity_key, key);
        assert_eq!(category_result.groups[0].count, 1);

        let property_result = store
            .query(
                &expected.snapshot_id,
                &SemanticQuery {
                    filters: vec![SemanticFilter::PropertyTextEquals {
                        name: "Comments".to_owned(),
                        value: "A".to_owned(),
                    }],
                    limit: 1,
                    ..SemanticQuery::default()
                },
            )
            .await
            .expect("property query succeeds");
        assert_eq!(property_result.total, 1);
        assert!(!property_result.has_more);
    });
}
