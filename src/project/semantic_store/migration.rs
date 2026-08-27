//! Turso schema migration entry point.

use anyhow::{Context, Result};

use super::schema::SCHEMA_SQL;

pub(crate) async fn apply(connection: &turso::Connection) -> Result<()> {
    connection
        .execute_batch(SCHEMA_SQL)
        .await
        .context("applying durable semantic-store schema")?;
    let mut rows = connection
        .query("PRAGMA table_info(properties)", ())
        .await
        .context("reading durable semantic property schema")?;
    let mut columns = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .context("reading durable semantic property schema row")?
    {
        columns.push(
            row.get::<String>(1)
                .context("decoding durable semantic property column")?,
        );
    }
    for column in ["quantity_id", "canonical_unit_id", "source_unit_id"] {
        if !columns.iter().any(|existing| existing == column) {
            connection
                .execute(
                    &format!("ALTER TABLE properties ADD COLUMN {column} TEXT"),
                    (),
                )
                .await
                .with_context(|| format!("adding durable semantic property column {column}"))?;
        }
    }
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (3)",
            (),
        )
        .await
        .context("recording durable semantic schema version")?;
    Ok(())
}
