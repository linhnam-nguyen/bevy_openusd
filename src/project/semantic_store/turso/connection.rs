use anyhow::{Context, Result};
use std::path::Path;

use crate::project::semantic_store::migration;

pub(crate) struct TursoSemanticStore {
    pub(super) _database: turso::Database,
    pub(super) connection: turso::Connection,
}

impl TursoSemanticStore {
    pub(crate) async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path != Path::new(":memory:") {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("creating Turso parent directory {}", parent.display())
                })?;
            }
        }
        let path_string = path.to_string_lossy().into_owned();
        let database = turso::Builder::new_local(&path_string)
            .build()
            .await
            .with_context(|| format!("opening Turso semantic store at {}", path.display()))?;
        let connection = database
            .connect()
            .context("connecting to Turso semantic store")?;
        migration::apply(&connection).await?;
        Ok(Self {
            _database: database,
            connection,
        })
    }

    pub(crate) async fn open_memory() -> Result<Self> {
        Self::open(":memory:").await
    }

    #[cfg(test)]
    pub(crate) fn connection(&self) -> &turso::Connection {
        &self.connection
    }
}
