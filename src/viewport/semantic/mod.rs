//! Working semantic query service backed by an in-memory Turso database.
//!
//! This module is intentionally not wired into `ViewportCommand::SearchScene`
//! yet. `SceneQueryService` remains the active migration path until the
//! semantic result shape is integrated with scene-anchor reveal paging.

mod query;
mod store;

use std::sync::{Mutex, mpsc};

use bevy::prelude::Resource;
use usd_model::SemanticSnapshot;

pub(crate) use query::{GroupField, SemanticFilter, SemanticQuery, SemanticQueryResult};

use store::SemanticDatabase;

#[derive(Debug)]
enum SemanticCommand {
    ReplaceSnapshot {
        request_id: String,
        snapshot: SemanticSnapshot,
    },
    Query {
        request_id: String,
        query: SemanticQuery,
    },
}

#[derive(Debug)]
pub(crate) enum SemanticResponse {
    SnapshotLoaded {
        request_id: String,
        entity_count: u32,
    },
    QueryResult {
        request_id: String,
        result: SemanticQueryResult,
    },
    Failed {
        request_id: String,
        operation: &'static str,
        error: String,
    },
}

/// The Bevy-facing channel endpoint for the dedicated semantic worker.
#[derive(Resource, Debug)]
pub(crate) struct SemanticWorkingStore {
    commands: mpsc::Sender<SemanticCommand>,
    responses: Mutex<mpsc::Receiver<SemanticResponse>>,
}

impl Default for SemanticWorkingStore {
    fn default() -> Self {
        let (commands, pending_commands) = mpsc::channel();
        let (responses, pending_responses) = mpsc::channel();
        std::thread::Builder::new()
            .name("usdview-semantic-worker".to_owned())
            .spawn(move || semantic_worker(pending_commands, responses))
            .expect("semantic worker should start");
        Self {
            commands,
            responses: Mutex::new(pending_responses),
        }
    }
}

impl SemanticWorkingStore {
    pub(crate) fn submit_snapshot(
        &self,
        request_id: impl Into<String>,
        snapshot: SemanticSnapshot,
    ) -> bool {
        self.commands
            .send(SemanticCommand::ReplaceSnapshot {
                request_id: request_id.into(),
                snapshot,
            })
            .is_ok()
    }

    pub(crate) fn submit_query(&self, request_id: impl Into<String>, query: SemanticQuery) -> bool {
        self.commands
            .send(SemanticCommand::Query {
                request_id: request_id.into(),
                query,
            })
            .is_ok()
    }

    pub(crate) fn drain_responses(&self) -> Vec<SemanticResponse> {
        let Ok(responses) = self.responses.lock() else {
            return Vec::new();
        };
        responses.try_iter().collect()
    }
}

fn semantic_worker(
    pending_commands: mpsc::Receiver<SemanticCommand>,
    responses: mpsc::Sender<SemanticResponse>,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("semantic worker runtime should build");
    let mut database = runtime.block_on(SemanticDatabase::open()).ok();

    while let Ok(command) = pending_commands.recv() {
        let (request_id, result, operation) = match command {
            SemanticCommand::ReplaceSnapshot {
                request_id,
                snapshot,
            } => {
                let result = database.as_mut().map_or_else(
                    || Err("semantic database is unavailable".to_owned()),
                    |database| {
                        runtime
                            .block_on(database.replace_snapshot(&snapshot))
                            .map_err(|error| error.to_string())
                    },
                );
                (
                    request_id,
                    result.map(|count| SemanticResponse::SnapshotLoaded {
                        request_id: String::new(),
                        entity_count: count,
                    }),
                    "snapshot load",
                )
            }
            SemanticCommand::Query { request_id, query } => {
                let result = database.as_ref().map_or_else(
                    || Err("semantic database is unavailable".to_owned()),
                    |database| {
                        runtime
                            .block_on(database.query(&query))
                            .map_err(|error| error.to_string())
                    },
                );
                (
                    request_id,
                    result.map(|result| SemanticResponse::QueryResult {
                        request_id: String::new(),
                        result,
                    }),
                    "query",
                )
            }
        };

        let response = match result {
            Ok(mut response) => {
                match &mut response {
                    SemanticResponse::SnapshotLoaded {
                        request_id: response_id,
                        ..
                    }
                    | SemanticResponse::QueryResult {
                        request_id: response_id,
                        ..
                    } => *response_id = request_id,
                    SemanticResponse::Failed { .. } => {}
                }
                response
            }
            Err(error) => SemanticResponse::Failed {
                request_id,
                operation,
                error,
            },
        };
        if responses.send(response).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use openusd::usd::Stage;
    use usd_model::{EntityKey, SemanticSnapshot, SnapshotSource};
    use usd_semantic::{SemanticConfig, SemanticExtractor};

    use super::{SemanticFilter, SemanticQuery, SemanticResponse, SemanticWorkingStore};

    fn snapshot() -> Result<SemanticSnapshot> {
        let stage = Stage::open("tests/stages/custom_attrs_extensive.usda")?;
        SemanticExtractor::new(SemanticConfig::default()).extract(
            &stage,
            SnapshotSource::Working {
                session: "semantic-worker-test".to_owned(),
                live_revision: 1,
            },
        )
    }

    fn response(store: &SemanticWorkingStore) -> SemanticResponse {
        for _ in 0..200 {
            if let Some(response) = store.drain_responses().into_iter().next() {
                return response;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("semantic worker did not respond")
    }

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
                group_by: vec![super::GroupField::TypeName],
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
}
