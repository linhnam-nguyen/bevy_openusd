use anyhow::{Result, bail};

use super::publish::{MigrationPublishCheckpoint, MigrationPublishObserver};

#[cfg_attr(windows, allow(dead_code))]
pub(super) struct InjectFailureObserver {
    checkpoint: MigrationPublishCheckpoint,
}

impl MigrationPublishObserver for InjectFailureObserver {
    fn on_checkpoint(&self, checkpoint: MigrationPublishCheckpoint) -> Result<()> {
        if checkpoint == self.checkpoint {
            bail!("injected Project migration failure at {checkpoint:?}");
        }
        Ok(())
    }
}

#[cfg_attr(windows, allow(dead_code))]
pub(super) fn at(checkpoint: MigrationPublishCheckpoint) -> InjectFailureObserver {
    InjectFailureObserver { checkpoint }
}
