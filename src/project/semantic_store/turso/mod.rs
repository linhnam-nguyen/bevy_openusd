//! Turso implementation of the durable semantic-store contract.

mod connection;
mod entity;
mod query;
mod snapshot;

pub(crate) use connection::TursoSemanticStore;

use anyhow::Result;
use usd_model::{EntityKey, EntitySnapshot, SemanticSnapshot, SnapshotId};

use super::SemanticStore;
use super::query::{SemanticQuery, SemanticQueryResult};

impl SemanticStore for TursoSemanticStore {
    async fn put_snapshot(&mut self, snapshot: &SemanticSnapshot) -> Result<()> {
        self.put_snapshot_impl(snapshot).await
    }

    async fn get_snapshot(&self, id: &SnapshotId) -> Result<Option<SemanticSnapshot>> {
        self.get_snapshot_impl(id).await
    }

    async fn get_entity(
        &self,
        snapshot: &SnapshotId,
        key: &EntityKey,
    ) -> Result<Option<EntitySnapshot>> {
        self.get_entity_impl(snapshot, key).await
    }

    async fn get_commit_snapshot(&self, git_oid: &str) -> Result<Option<SemanticSnapshot>> {
        self.get_commit_snapshot_impl(git_oid).await
    }

    async fn query(
        &self,
        snapshot: &SnapshotId,
        query: &SemanticQuery,
    ) -> Result<SemanticQueryResult> {
        self.query_impl(snapshot, query).await
    }
}

#[cfg(test)]
mod tests;
