//! Turso schema migration entry point.

use anyhow::{Context, Result};

use super::schema::SCHEMA_SQL;

pub(crate) async fn apply(connection: &turso::Connection) -> Result<()> {
    connection
        .execute_batch(SCHEMA_SQL)
        .await
        .context("applying durable semantic-store schema")?;
    Ok(())
}
