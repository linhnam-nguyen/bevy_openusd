use anyhow::{Context, Result, bail};
use std::{collections::HashMap, sync::Mutex};
use viewport_protocol::{AuthorizationPolicy, SessionId};

use super::client_config::{TursoClientSyncConfig, TursoCloudProvisioningConfig};
use super::platform_api::TursoCloudAdmin;

/// Application request for one isolated client/session sync database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TursoClientSyncProvisionRequest {
    pub session_id: SessionId,
    pub client_name: String,
    pub authorization: AuthorizationPolicy,
}

/// Opaque credentials returned by a provisioning provider.
///
/// The token is intentionally private, does not implement `Debug`, and is
/// never represented in the viewport protocol status.
pub(crate) struct TursoClientSyncCredentials {
    pub(super) config: TursoClientSyncConfig,
    pub(super) auth_token: String,
}

impl TursoClientSyncCredentials {
    pub(crate) fn new(config: TursoClientSyncConfig, auth_token: String) -> Result<Self> {
        config.validate(&auth_token)?;
        Ok(Self { config, auth_token })
    }
}

/// Server-owned boundary for Turso Cloud database provisioning and revocation.
///
/// A concrete provider may call a deployment-specific administration API, but
/// the semantic-store coordinator does not assume or invent that API.
pub(crate) trait TursoClientSyncProvisioner: Send + Sync {
    fn provision(
        &self,
        request: &TursoClientSyncProvisionRequest,
    ) -> Result<TursoClientSyncCredentials>;

    fn revoke(&self, session_id: &SessionId) -> Result<()>;
}

/// Turso Cloud implementation of [`TursoClientSyncProvisioner`].
///
/// One provider instance owns the server-side lease map. The returned
/// credentials contain the client database URL and scoped token, but the
/// provider never serializes or publishes them; the coordinator keeps them
/// opaque until the local sync client is opened.
pub(crate) struct TursoCloudProvisioner<A> {
    admin: A,
    config: TursoCloudProvisioningConfig,
    leases: Mutex<HashMap<SessionId, TursoCloudLease>>,
}

struct TursoCloudLease {
    database_name: String,
}

impl<A> TursoCloudProvisioner<A>
where
    A: TursoCloudAdmin,
{
    pub(crate) fn new(admin: A, config: TursoCloudProvisioningConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            admin,
            config,
            leases: Mutex::new(HashMap::new()),
        })
    }

    fn database_name(&self, request: &TursoClientSyncProvisionRequest) -> String {
        let identity = format!("{}:{}", request.session_id.0, request.client_name);
        let suffix = blake3::hash(identity.as_bytes()).to_hex();
        format!("{}-{}", self.config.database_prefix, &suffix[..20])
    }

    fn delete_created_database(&self, database_name: &str, error: anyhow::Error) -> anyhow::Error {
        match self
            .admin
            .delete_database(&self.config.organization_slug, database_name)
        {
            Ok(()) => error,
            Err(cleanup_error) => error.context(format!(
                "failed to clean up Turso database after provisioning error: {cleanup_error:?}"
            )),
        }
    }
}

impl<A> TursoClientSyncProvisioner for TursoCloudProvisioner<A>
where
    A: TursoCloudAdmin,
{
    fn provision(
        &self,
        request: &TursoClientSyncProvisionRequest,
    ) -> Result<TursoClientSyncCredentials> {
        let mut leases = self
            .leases
            .lock()
            .expect("Turso Cloud lease map should not be poisoned");
        if leases.contains_key(&request.session_id) {
            bail!("Turso client lease already exists for this session");
        }

        let database_name = self.database_name(request);
        let database = self
            .admin
            .create_database(
                &self.config.organization_slug,
                &database_name,
                &self.config.group_name,
            )
            .context("creating isolated Turso client database")?;
        let hostname = database.hostname.trim();
        if hostname.is_empty()
            || hostname.chars().any(char::is_whitespace)
            || hostname.contains('/')
            || hostname.contains("://")
        {
            let error = anyhow::anyhow!("Turso Platform API returned an invalid database hostname");
            return Err(self.delete_created_database(&database_name, error));
        }

        let token = match self.admin.create_database_token(
            &self.config.organization_slug,
            &database_name,
            self.config.token_expiration.as_deref(),
        ) {
            Ok(token) => token,
            Err(error) => return Err(self.delete_created_database(&database_name, error)),
        };
        let credentials = match TursoClientSyncCredentials::new(
            TursoClientSyncConfig {
                local_path: self.config.local_root.join(format!("{database_name}.db")),
                remote_url: format!("libsql://{hostname}"),
                client_name: request.client_name.clone(),
            },
            token,
        ) {
            Ok(credentials) => credentials,
            Err(error) => return Err(self.delete_created_database(&database_name, error)),
        };

        leases.insert(
            request.session_id.clone(),
            TursoCloudLease { database_name },
        );
        Ok(credentials)
    }

    fn revoke(&self, session_id: &SessionId) -> Result<()> {
        let mut leases = self
            .leases
            .lock()
            .expect("Turso Cloud lease map should not be poisoned");
        let Some(lease) = leases.get(session_id) else {
            return Ok(());
        };
        self.admin
            .delete_database(&self.config.organization_slug, &lease.database_name)
            .context("deleting isolated Turso client database")?;
        leases.remove(session_id);
        Ok(())
    }
}
