//! Turso schema, snapshot bulk loading, and parameterized semantic queries.

mod delta;
mod query;
mod row;
mod schema;
mod snapshot;

pub(crate) use schema::SCHEMA_VERSION;

use anyhow::{Context, Result};

use schema::SCHEMA_SQL;

pub(crate) struct SemanticDatabase {
    _database: turso::Database,
    connection: turso::Connection,
}

impl SemanticDatabase {
    pub(crate) async fn open() -> Result<Self> {
        let database = turso::Builder::new_local(":memory:")
            .build()
            .await
            .context("opening in-memory Turso database")?;
        let connection = database
            .connect()
            .context("connecting to in-memory Turso database")?;
        connection
            .execute_batch(SCHEMA_SQL)
            .await
            .context("applying semantic Turso schema")?;
        Ok(Self {
            _database: database,
            connection,
        })
    }
}

#[cfg(test)]
mod tests;
