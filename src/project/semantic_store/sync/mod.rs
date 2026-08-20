//! Authorization-safe semantic projections for client synchronization.

mod client;
mod client_config;
mod coordinator;
mod lifecycle;
mod platform_api;
mod projection;
mod provisioning;

pub(crate) use client::TursoClientSync;
pub(crate) use client_config::{TursoClientSyncConfig, TursoCloudProvisioningConfig};
pub(crate) use coordinator::{TursoClientSyncCoordinator, TursoClientSyncUpdate};
pub(crate) use lifecycle::{
    MAX_PENDING_SYNC_RUNTIME_COMMANDS, RuntimeMailbox, TursoClientSyncApplication,
    TursoClientSyncRuntime, TursoClientSyncRuntimeCommand, TursoClientSyncRuntimeSubmitError,
};
pub(crate) use platform_api::{
    TursoCloudAdmin, TursoCloudDatabase, TursoPlatformApi, TursoPlatformApiConfig,
};
pub(crate) use projection::{
    AuthorizedEntitySnapshot, AuthorizedSemanticSnapshot, authorize_snapshot,
};
pub(crate) use provisioning::{
    TursoClientSyncCredentials, TursoClientSyncProvisionRequest, TursoClientSyncProvisioner,
    TursoCloudProvisioner,
};

#[cfg(test)]
mod tests;
