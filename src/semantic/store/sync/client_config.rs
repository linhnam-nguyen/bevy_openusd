use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use viewport_protocol::AuthorizationPolicy;

/// Configuration for one dedicated per-client Turso database.
///
/// The token is intentionally not stored in this value so it cannot be
/// serialized with sync diagnostics or accidentally logged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TursoClientSyncConfig {
    pub local_path: PathBuf,
    pub remote_url: String,
    pub client_name: String,
}

impl TursoClientSyncConfig {
    pub(crate) fn validate(&self, auth_token: &str) -> Result<()> {
        if self.local_path.as_os_str().is_empty() {
            bail!("client Turso sync local path must not be empty");
        }
        if self.remote_url.trim().is_empty() {
            bail!("client Turso sync remote URL must not be empty");
        }
        if self.client_name.trim().is_empty() {
            bail!("client Turso sync client name must not be empty");
        }
        if auth_token.trim().is_empty() {
            bail!("client Turso sync auth token must not be empty");
        }
        Ok(())
    }
}

/// Deployment settings for one Turso Cloud database-per-client provider.
///
/// The platform token is held by the injected [`super::platform_api::TursoCloudAdmin`] implementation
/// rather than this value, keeping configuration and diagnostics credential-free.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TursoCloudProvisioningConfig {
    pub organization_slug: String,
    pub group_name: String,
    pub database_prefix: String,
    pub local_root: PathBuf,
    pub token_expiration: Option<String>,
}

impl TursoCloudProvisioningConfig {
    pub(crate) fn from_environment() -> Result<Self> {
        let organization_slug = std::env::var("TURSO_ORGANIZATION")
            .context("TURSO_ORGANIZATION must be set for Turso semantic sync")?;
        let group_name =
            std::env::var("TURSO_CLIENT_GROUP").unwrap_or_else(|_| "default".to_owned());
        let database_prefix = std::env::var("TURSO_CLIENT_DATABASE_PREFIX")
            .unwrap_or_else(|_| "usdhub-client".to_owned());
        let local_root = std::env::var_os("TURSO_CLIENT_SYNC_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("usdhub-client-sync"));
        let token_expiration = std::env::var("TURSO_CLIENT_TOKEN_EXPIRATION").ok();
        Self::validate(&Self {
            organization_slug: organization_slug.clone(),
            group_name: group_name.clone(),
            database_prefix: database_prefix.clone(),
            local_root: local_root.clone(),
            token_expiration: token_expiration.clone(),
        })?;
        Ok(Self {
            organization_slug,
            group_name,
            database_prefix,
            local_root,
            token_expiration,
        })
    }

    pub(crate) fn validate(&self) -> Result<()> {
        validate_turso_slug(&self.organization_slug, "organization slug")?;
        validate_turso_slug(&self.group_name, "group name")?;
        validate_turso_slug(&self.database_prefix, "database prefix")?;
        if self.database_prefix.len() > 32 {
            bail!("Turso database prefix must be at most 32 characters");
        }
        if self.local_root.as_os_str().is_empty() {
            bail!("Turso client local root must not be empty");
        }
        if self
            .token_expiration
            .as_deref()
            .is_some_and(|expiration| expiration.trim().is_empty())
        {
            bail!("Turso client token expiration must not be empty");
        }
        Ok(())
    }
}

pub(super) fn validate_turso_slug(value: &str, field: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || value.starts_with('-')
        || value.ends_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        bail!("Turso {field} must use lowercase letters, numbers, and dashes");
    }
    Ok(())
}

pub(super) fn require_self_render_sync(authorization: &AuthorizationPolicy) -> Result<()> {
    if !authorization.allows_self_render_delivery() {
        bail!("semantic-sync requires self-render delivery authorization");
    }
    if !authorization.allows_model_download() {
        bail!("semantic-sync requires model-download authorization");
    }
    Ok(())
}
