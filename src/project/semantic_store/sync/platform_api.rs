use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::client_config::validate_turso_slug;

/// Database identity returned by the Turso Platform API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TursoCloudDatabase {
    pub hostname: String,
}

/// Minimal administration contract required by the database-per-client
/// provider. An HTTP implementation can map these calls to the official
/// Turso Platform API without exposing API details to the session coordinator.
pub(crate) trait TursoCloudAdmin: Send + Sync {
    fn create_database(
        &self,
        organization_slug: &str,
        database_name: &str,
        group_name: &str,
    ) -> Result<TursoCloudDatabase>;

    fn create_database_token(
        &self,
        organization_slug: &str,
        database_name: &str,
        expiration: Option<&str>,
    ) -> Result<String>;

    fn delete_database(&self, organization_slug: &str, database_name: &str) -> Result<()>;
}

/// Configuration for the server-only Turso Platform API client.
///
/// The platform token is intentionally private and this type does not derive
/// `Debug`, `Serialize`, or `Deserialize`.
pub(crate) struct TursoPlatformApiConfig {
    pub api_base_url: String,
    pub organization_slug: String,
    platform_token: String,
}

impl TursoPlatformApiConfig {
    pub(crate) fn new(
        api_base_url: String,
        organization_slug: String,
        platform_token: String,
    ) -> Result<Self> {
        let url = reqwest::Url::parse(&api_base_url).context("parsing Turso Platform API URL")?;
        if !matches!(url.scheme(), "http" | "https") {
            bail!("Turso Platform API URL must use http or https");
        }
        validate_turso_slug(&organization_slug, "organization slug")?;
        if platform_token.trim().is_empty() {
            bail!("Turso Platform API token must not be empty");
        }
        Ok(Self {
            api_base_url: api_base_url.trim_end_matches('/').to_owned(),
            organization_slug,
            platform_token,
        })
    }

    pub(crate) fn from_environment() -> Result<Self> {
        let api_base_url = std::env::var("TURSO_PLATFORM_API_URL")
            .unwrap_or_else(|_| "https://api.turso.tech/v1".to_owned());
        let organization_slug = std::env::var("TURSO_ORGANIZATION")
            .context("TURSO_ORGANIZATION must be set for Turso semantic sync")?;
        let platform_token = std::env::var("TURSO_PLATFORM_TOKEN")
            .context("TURSO_PLATFORM_TOKEN must be set for Turso semantic sync")?;
        Self::new(api_base_url, organization_slug, platform_token)
    }
}

/// Blocking HTTP transport for the official Turso Platform API.
///
/// The session coordinator calls provisioning synchronously, so this adapter
/// uses reqwest's blocking client. It is intended to run on the application
/// control thread, never inside the WebRTC media path.
pub(crate) struct TursoPlatformApi {
    client: reqwest::blocking::Client,
    config: TursoPlatformApiConfig,
}

impl TursoPlatformApi {
    pub(crate) fn new(config: TursoPlatformApiConfig) -> Result<Self> {
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .build()
                .context("building Turso Platform API HTTP client")?,
            config,
        })
    }

    pub(crate) fn from_environment() -> Result<Self> {
        Self::new(TursoPlatformApiConfig::from_environment()?)
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{path}", self.config.api_base_url)
    }

    fn ensure_success(
        response: reqwest::blocking::Response,
        operation: &str,
    ) -> Result<reqwest::blocking::Response> {
        let status = response.status();
        if !status.is_success() {
            bail!("Turso Platform API {operation} failed with HTTP {status}");
        }
        Ok(response)
    }
}

#[derive(Serialize)]
struct CreateDatabaseRequest<'a> {
    name: &'a str,
    group: &'a str,
}

#[derive(Deserialize)]
pub(super) struct CreateDatabaseResponse {
    pub(super) database: CreateDatabasePayload,
}

#[derive(Deserialize)]
pub(super) struct CreateDatabasePayload {
    #[serde(alias = "Hostname")]
    pub(super) hostname: String,
}

#[derive(Deserialize)]
pub(super) struct CreateTokenResponse {
    pub(super) jwt: String,
}

impl TursoCloudAdmin for TursoPlatformApi {
    fn create_database(
        &self,
        organization_slug: &str,
        database_name: &str,
        group_name: &str,
    ) -> Result<TursoCloudDatabase> {
        let response = self
            .client
            .post(self.endpoint(&format!("organizations/{organization_slug}/databases")))
            .bearer_auth(&self.config.platform_token)
            .json(&CreateDatabaseRequest {
                name: database_name,
                group: group_name,
            })
            .send()
            .context("calling Turso database creation API")?;
        let response = Self::ensure_success(response, "database creation")?;
        let payload = response
            .json::<CreateDatabaseResponse>()
            .context("decoding Turso database creation response")?;
        Ok(TursoCloudDatabase {
            hostname: payload.database.hostname,
        })
    }

    fn create_database_token(
        &self,
        organization_slug: &str,
        database_name: &str,
        expiration: Option<&str>,
    ) -> Result<String> {
        let mut query = vec![("authorization", "full-access".to_owned())];
        if let Some(expiration) = expiration {
            query.push(("expiration", expiration.to_owned()));
        }
        let response = self
            .client
            .post(self.endpoint(&format!(
                "organizations/{organization_slug}/databases/{database_name}/auth/tokens"
            )))
            .bearer_auth(&self.config.platform_token)
            .query(&query)
            .send()
            .context("calling Turso database token API")?;
        let response = Self::ensure_success(response, "database token creation")?;
        let payload = response
            .json::<CreateTokenResponse>()
            .context("decoding Turso database token response")?;
        if payload.jwt.trim().is_empty() {
            bail!("Turso database token response contained an empty token");
        }
        Ok(payload.jwt)
    }

    fn delete_database(&self, organization_slug: &str, database_name: &str) -> Result<()> {
        let response = self
            .client
            .delete(self.endpoint(&format!(
                "organizations/{organization_slug}/databases/{database_name}"
            )))
            .bearer_auth(&self.config.platform_token)
            .send()
            .context("calling Turso database deletion API")?;
        Self::ensure_success(response, "database deletion")?;
        Ok(())
    }
}
