use anyhow::{Context, Result, bail};

use super::client_config::TursoClientSyncConfig;
use super::projection::{AuthorizedSemanticSnapshot, verify_projection_hash};

const CLIENT_PROJECTION_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS authorized_semantic_projection (
    projection_hash   TEXT PRIMARY KEY NOT NULL,
    source_snapshot_id TEXT NOT NULL,
    projection_json   TEXT NOT NULL
);
"#;

/// Explicit lifecycle wrapper around Turso Cloud synchronization for one
/// authorized semantic projection.
#[allow(dead_code)]
pub(crate) struct TursoClientSync {
    database: turso::sync::Database,
    connection: turso::Connection,
}

#[allow(dead_code)]
impl TursoClientSync {
    /// Opens a dedicated synced database without bootstrapping arbitrary
    /// remote tables. The caller must provision the remote database for this
    /// client/session before using `push_projection` or `pull_projection`.
    pub(crate) async fn open(config: &TursoClientSyncConfig, auth_token: &str) -> Result<Self> {
        config.validate(auth_token)?;
        let local_path = config.local_path.to_string_lossy();
        let database = turso::sync::Builder::new_remote(local_path.as_ref())
            .with_remote_url(config.remote_url.clone())
            .with_auth_token(auth_token.to_owned())
            .with_client_name(config.client_name.clone())
            .bootstrap_if_empty(false)
            .build()
            .await
            .context("opening dedicated client Turso sync database")?;
        let connection = database
            .connect()
            .await
            .context("connecting to dedicated client Turso sync database")?;
        initialize_client_projection_store(&connection).await?;
        Ok(Self {
            database,
            connection,
        })
    }

    /// Replaces the local projection and pushes only that projection to the
    /// dedicated remote database.
    pub(crate) async fn push_projection(
        &mut self,
        projection: &AuthorizedSemanticSnapshot,
    ) -> Result<()> {
        replace_client_projection(&mut self.connection, projection).await?;
        self.database
            .push()
            .await
            .context("pushing authorized semantic projection to Turso")
    }

    /// Pulls remote changes and returns one verified authorized projection.
    /// Multiple rows or a mismatched projection hash fail closed.
    pub(crate) async fn pull_projection(&self) -> Result<Option<AuthorizedSemanticSnapshot>> {
        self.database
            .pull()
            .await
            .context("pulling authorized semantic projection from Turso")?;
        read_client_projection(&self.connection).await
    }

    pub(crate) async fn stats(&self) -> Result<turso::sync::DatabaseSyncStats> {
        self.database
            .stats()
            .await
            .context("reading Turso client-sync statistics")
    }
}

pub(super) async fn initialize_client_projection_store(
    connection: &turso::Connection,
) -> Result<()> {
    connection
        .execute_batch(CLIENT_PROJECTION_SCHEMA)
        .await
        .context("creating authorized client projection table")?;
    Ok(())
}

pub(super) async fn replace_client_projection(
    connection: &mut turso::Connection,
    projection: &AuthorizedSemanticSnapshot,
) -> Result<()> {
    verify_projection_hash(projection)?;
    let payload = serde_json::to_string(projection)
        .context("serializing authorized client semantic projection")?;
    let transaction = connection
        .transaction()
        .await
        .context("starting client projection replacement transaction")?;
    transaction
        .execute("DELETE FROM authorized_semantic_projection", ())
        .await
        .context("clearing previous client semantic projection")?;
    transaction
        .execute(
            "INSERT INTO authorized_semantic_projection
                (projection_hash, source_snapshot_id, projection_json)
             VALUES (?1, ?2, ?3)",
            turso::params![
                projection.projection_hash.to_hex(),
                projection.source_snapshot_id.0.clone(),
                payload,
            ],
        )
        .await
        .context("storing authorized client semantic projection")?;
    transaction
        .commit()
        .await
        .context("committing client semantic projection replacement")?;
    Ok(())
}

pub(super) async fn read_client_projection(
    connection: &turso::Connection,
) -> Result<Option<AuthorizedSemanticSnapshot>> {
    let mut rows = connection
        .query(
            "SELECT projection_hash, projection_json
               FROM authorized_semantic_projection
              ORDER BY projection_hash",
            (),
        )
        .await
        .context("reading authorized client semantic projection")?;
    let Some(row) = rows
        .next()
        .await
        .context("reading authorized client projection row")?
    else {
        return Ok(None);
    };
    if rows
        .next()
        .await
        .context("checking client projection uniqueness")?
        .is_some()
    {
        bail!("client Turso database contains multiple semantic projections");
    }

    let stored_hash: String = row
        .get(0)
        .context("decoding stored client projection hash")?;
    let payload: String = row
        .get(1)
        .context("decoding stored client projection JSON")?;
    let projection: AuthorizedSemanticSnapshot = serde_json::from_str(&payload)
        .context("deserializing authorized client semantic projection")?;
    if projection.projection_hash.to_hex() != stored_hash {
        bail!("client projection row hash does not match its payload");
    }
    verify_projection_hash(&projection)?;
    Ok(Some(projection))
}
