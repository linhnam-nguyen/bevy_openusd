//! Authorization-safe semantic projections for future client synchronization.
//!
//! A remote client must never receive the complete server semantic database by
//! relying on Turso partial-sync filtering. This module creates an explicit
//! per-policy projection first; remote transport can only replicate that
//! projection later.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use usd_model::{
    EntityKey, GeometrySignature, HashDigest, IdentitySource, SemanticInfo, SemanticProperty,
    SemanticSnapshot, SnapshotId, SnapshotSource, TransformSignature,
};
use viewport_protocol::{AuthorizationPolicy, SemanticSyncPhase, SemanticSyncStatus, SessionId};

use super::{SemanticStore, TursoSemanticStore};

const CLIENT_PROJECTION_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS authorized_semantic_projection (
    projection_hash   TEXT PRIMARY KEY NOT NULL,
    source_snapshot_id TEXT NOT NULL,
    projection_json   TEXT NOT NULL
);
"#;

/// A semantic snapshot view authorized for one self-render client.
///
/// The source hashes remain available for provenance, while `projection_hash`
/// identifies the filtered view and must be used for client cache identity.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct AuthorizedSemanticSnapshot {
    pub source_snapshot_id: SnapshotId,
    pub source: SnapshotSource,
    pub config_hash: HashDigest,
    pub projection_hash: HashDigest,
    pub entities: BTreeMap<EntityKey, AuthorizedEntitySnapshot>,
}

/// One entity in an authorization-safe semantic snapshot view.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct AuthorizedEntitySnapshot {
    pub key: EntityKey,
    pub prim_path: String,
    pub identity_source: IdentitySource,
    pub semantic: SemanticInfo,
    pub transform: TransformSignature,
    pub geometry: Option<GeometrySignature>,
    pub properties: Vec<SemanticProperty>,
    pub source_metadata_hash: HashDigest,
    pub source_full_hash: HashDigest,
}

impl TursoSemanticStore {
    /// Reads one durable snapshot and returns only the data authorized for a
    /// self-render client. This is deliberately separate from remote Turso
    /// replication so policy filtering happens before any bytes leave the
    /// server database.
    #[allow(dead_code)]
    pub(crate) async fn get_authorized_snapshot(
        &self,
        snapshot_id: &SnapshotId,
        policy: &AuthorizationPolicy,
    ) -> Result<Option<AuthorizedSemanticSnapshot>> {
        let Some(snapshot) = self.get_snapshot(snapshot_id).await? else {
            return Ok(None);
        };
        authorize_snapshot(&snapshot, policy).map(Some)
    }
}

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
/// The platform token is held by the injected [`TursoCloudAdmin`] implementation
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
struct CreateDatabaseResponse {
    database: CreateDatabasePayload,
}

#[derive(Deserialize)]
struct CreateDatabasePayload {
    #[serde(alias = "Hostname")]
    hostname: String,
}

#[derive(Deserialize)]
struct CreateTokenResponse {
    jwt: String,
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

/// Turso Cloud implementation of [`TursoClientSyncProvisioner`].
///
/// One provider instance owns the server-side lease map. The returned
/// credentials contain the client database URL and scoped token, but the
/// provider never serializes or publishes them; the coordinator keeps them
/// opaque until the local sync client is opened.
pub(crate) struct TursoCloudProvisioner<A> {
    admin: A,
    config: TursoCloudProvisioningConfig,
    leases: std::sync::Mutex<HashMap<SessionId, TursoCloudLease>>,
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
            leases: std::sync::Mutex::new(HashMap::new()),
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

fn validate_turso_slug(value: &str, field: &str) -> Result<()> {
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

const MAX_SYNC_STATUS_UPDATES: usize = 256;

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
    config: TursoClientSyncConfig,
    auth_token: String,
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

/// One status update for the application/session event boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TursoClientSyncUpdate {
    pub session_id: SessionId,
    pub status: SemanticSyncStatus,
}

/// Root-application composition boundary for semantic synchronization.
///
/// The adapter owns the coordinator and publishes only its credential-free
/// status updates to the transport-neutral interface. It deliberately does not
/// extract snapshots or run a background task; the Bevy application decides
/// when to call each lifecycle operation.
#[allow(dead_code)]
pub(crate) struct TursoClientSyncApplication<P> {
    coordinator: TursoClientSyncCoordinator<P>,
    interface: viewport_streaming::RenderServerInterface,
}

#[allow(dead_code)]
impl<P> TursoClientSyncApplication<P>
where
    P: TursoClientSyncProvisioner,
{
    pub(crate) fn new(
        provisioner: P,
        interface: viewport_streaming::RenderServerInterface,
    ) -> Self {
        Self {
            coordinator: TursoClientSyncCoordinator::new(provisioner),
            interface,
        }
    }

    pub(crate) fn provision(&mut self, request: TursoClientSyncProvisionRequest) -> Result<()> {
        let result = self.coordinator.provision(request);
        self.publish_updates()?;
        result
    }

    pub(crate) async fn connect(&mut self, session_id: &SessionId) -> Result<()> {
        let result = self.coordinator.connect(session_id).await;
        self.publish_updates()?;
        result
    }

    pub(crate) async fn push_snapshot(
        &mut self,
        session_id: &SessionId,
        snapshot: &SemanticSnapshot,
    ) -> Result<AuthorizedSemanticSnapshot> {
        let result = self.coordinator.push_snapshot(session_id, snapshot).await;
        self.publish_updates()?;
        result
    }

    pub(crate) async fn pull_projection(
        &mut self,
        session_id: &SessionId,
    ) -> Result<Option<AuthorizedSemanticSnapshot>> {
        let result = self.coordinator.pull_projection(session_id).await;
        self.publish_updates()?;
        result
    }

    pub(crate) fn update_authorization(
        &mut self,
        session_id: &SessionId,
        authorization: AuthorizationPolicy,
    ) -> Result<()> {
        let result = self
            .coordinator
            .update_authorization(session_id, authorization);
        self.publish_updates()?;
        result
    }

    pub(crate) fn close(&mut self, session_id: &SessionId) -> Result<()> {
        let result = self.coordinator.close(session_id);
        self.publish_updates()?;
        result
    }

    pub(crate) fn status(&self, session_id: &SessionId) -> Option<SemanticSyncStatus> {
        self.coordinator.status(session_id)
    }

    fn publish_updates(&mut self) -> Result<()> {
        for update in self.coordinator.drain_updates() {
            self.interface
                .publish_semantic_sync_status(update.session_id, update.status)
                .map_err(|error| anyhow::anyhow!("publishing semantic-sync status: {error:?}"))?;
        }
        Ok(())
    }
}

impl TursoClientSyncApplication<TursoCloudProvisioner<TursoPlatformApi>> {
    /// Composes the production Platform API, lease provider, coordinator, and
    /// transport interface without starting any background work.
    #[allow(dead_code)]
    pub(crate) fn from_environment(
        interface: viewport_streaming::RenderServerInterface,
        provisioning: TursoCloudProvisioningConfig,
    ) -> Result<Self> {
        let provider =
            TursoCloudProvisioner::new(TursoPlatformApi::from_environment()?, provisioning)?;
        Ok(Self::new(provider, interface))
    }
}

/// Commands sent to the dedicated semantic-sync worker. The worker owns the
/// application coordinator so Bevy and WebRTC never block on Turso I/O.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TursoClientSyncRuntimeCommand {
    Provision(TursoClientSyncProvisionRequest),
    Connect(SessionId),
    PushSnapshot {
        session_id: SessionId,
        snapshot: SemanticSnapshot,
    },
    PullProjection(SessionId),
    UpdateAuthorization {
        session_id: SessionId,
        authorization: AuthorizationPolicy,
    },
    Close(SessionId),
}

impl TursoClientSyncRuntimeCommand {
    pub(crate) fn session_id(&self) -> &SessionId {
        match self {
            Self::Provision(request) => &request.session_id,
            Self::Connect(session_id) => session_id,
            Self::PushSnapshot { session_id, .. } => session_id,
            Self::PullProjection(session_id) => session_id,
            Self::UpdateAuthorization { session_id, .. } => session_id,
            Self::Close(session_id) => session_id,
        }
    }

    pub(crate) fn is_control(&self) -> bool {
        matches!(self, Self::UpdateAuthorization { .. } | Self::Close(_))
    }
}

/// Errors returned when submitting a command to the semantic-sync runtime mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TursoClientSyncRuntimeSubmitError {
    QueueFull,
    WorkerUnavailable,
    SessionClosed,
}

impl std::fmt::Display for TursoClientSyncRuntimeSubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueueFull => write!(f, "semantic-sync runtime mailbox queue is full"),
            Self::WorkerUnavailable => write!(f, "semantic-sync worker is unavailable"),
            Self::SessionClosed => write!(f, "semantic-sync session is closed"),
        }
    }
}

impl std::error::Error for TursoClientSyncRuntimeSubmitError {}

pub(crate) const MAX_PENDING_SYNC_RUNTIME_COMMANDS: usize = 64;

#[derive(Default)]
struct RuntimeMailboxState {
    normal: VecDeque<TursoClientSyncRuntimeCommand>,
    controls: HashMap<SessionId, TursoClientSyncRuntimeCommand>,
    closed_sessions: HashSet<SessionId>,
    closed: bool,
}

#[derive(Clone)]
pub(crate) struct RuntimeMailbox {
    state: Arc<(Mutex<RuntimeMailboxState>, Condvar)>,
}

impl RuntimeMailbox {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new((Mutex::new(RuntimeMailboxState::default()), Condvar::new())),
        }
    }

    pub(crate) fn submit(
        &self,
        command: TursoClientSyncRuntimeCommand,
    ) -> std::result::Result<(), TursoClientSyncRuntimeSubmitError> {
        let (lock, cvar) = &*self.state;
        let mut state = lock
            .lock()
            .map_err(|_| TursoClientSyncRuntimeSubmitError::WorkerUnavailable)?;

        if state.closed {
            return Err(TursoClientSyncRuntimeSubmitError::WorkerUnavailable);
        }

        let session_id = command.session_id().clone();

        if command.is_control() {
            let is_close = matches!(command, TursoClientSyncRuntimeCommand::Close(_));
            if is_close {
                state.closed_sessions.insert(session_id.clone());
            }

            let should_insert = match state.controls.get(&session_id) {
                Some(existing) => match (existing, &command) {
                    (
                        TursoClientSyncRuntimeCommand::Close(_),
                        TursoClientSyncRuntimeCommand::UpdateAuthorization { .. },
                    ) => false,
                    _ => true,
                },
                None => true,
            };

            if should_insert {
                state.controls.insert(session_id.clone(), command);
            }

            // ALWAYS purge same-session ordinary work whenever
            // a valid lifecycle control was submitted.
            state.normal.retain(|cmd| cmd.session_id() != &session_id);

            cvar.notify_one();
            Ok(())
        } else {
            if state.closed_sessions.contains(&session_id) {
                return Err(TursoClientSyncRuntimeSubmitError::SessionClosed);
            }
            if state.normal.len() >= MAX_PENDING_SYNC_RUNTIME_COMMANDS {
                return Err(TursoClientSyncRuntimeSubmitError::QueueFull);
            }
            state.normal.push_back(command);
            cvar.notify_one();
            Ok(())
        }
    }

    pub(crate) fn pop(&self) -> Option<TursoClientSyncRuntimeCommand> {
        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().ok()?;
        loop {
            if let Some(session_id) = state.controls.keys().next().cloned() {
                return state.controls.remove(&session_id);
            }
            if let Some(normal_cmd) = state.normal.pop_front() {
                return Some(normal_cmd);
            }
            if state.closed {
                return None;
            }
            state = cvar.wait(state).ok()?;
        }
    }

    pub(crate) fn close(&self) {
        let (lock, cvar) = &*self.state;
        if let Ok(mut state) = lock.lock() {
            state.closed = true;
            cvar.notify_all();
        }
    }
}

/// Opt-in application worker for explicit semantic-sync lifecycle requests.
#[derive(Clone)]
pub(crate) struct TursoClientSyncRuntime {
    mailbox: RuntimeMailbox,
}

impl TursoClientSyncRuntime {
    pub(crate) fn from_environment(
        interface: viewport_streaming::RenderServerInterface,
    ) -> Result<Option<Self>> {
        let enabled = std::env::var("TURSO_SEMANTIC_SYNC_ENABLED")
            .ok()
            .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "on"));
        if !enabled {
            return Ok(None);
        }

        let provisioning = TursoCloudProvisioningConfig::from_environment()?;
        let application = TursoClientSyncApplication::from_environment(interface, provisioning)?;
        let mailbox = RuntimeMailbox::new();
        let worker_mailbox = mailbox.clone();
        std::thread::Builder::new()
            .name("usdhub-semantic-sync".to_owned())
            .spawn(move || semantic_sync_worker(application, worker_mailbox))
            .context("starting semantic-sync worker")?;
        Ok(Some(Self { mailbox }))
    }

    pub(crate) fn submit(
        &self,
        command: TursoClientSyncRuntimeCommand,
    ) -> std::result::Result<(), TursoClientSyncRuntimeSubmitError> {
        self.mailbox.submit(command)
    }
}

impl Drop for TursoClientSyncRuntime {
    fn drop(&mut self) {
        if Arc::strong_count(&self.mailbox.state) <= 2 {
            self.mailbox.close();
        }
    }
}

struct RuntimeMailboxWorkerGuard {
    mailbox: RuntimeMailbox,
}

impl Drop for RuntimeMailboxWorkerGuard {
    fn drop(&mut self) {
        self.mailbox.close();
    }
}

fn semantic_sync_worker(
    mut application: TursoClientSyncApplication<TursoCloudProvisioner<TursoPlatformApi>>,
    mailbox: RuntimeMailbox,
) {
    let worker_guard = RuntimeMailboxWorkerGuard { mailbox };

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log::error!("[semantic-sync] worker runtime failed to start: {error:#}");
            return;
        }
    };

    while let Some(command) = worker_guard.mailbox.pop() {
        let result = match command {
            TursoClientSyncRuntimeCommand::Provision(request) => application.provision(request),
            TursoClientSyncRuntimeCommand::Connect(session_id) => {
                runtime.block_on(application.connect(&session_id))
            }
            TursoClientSyncRuntimeCommand::PushSnapshot {
                session_id,
                snapshot,
            } => runtime
                .block_on(application.push_snapshot(&session_id, &snapshot))
                .map(|_| ()),
            TursoClientSyncRuntimeCommand::PullProjection(session_id) => runtime
                .block_on(application.pull_projection(&session_id))
                .map(|_| ()),
            TursoClientSyncRuntimeCommand::UpdateAuthorization {
                session_id,
                authorization,
            } => application.update_authorization(&session_id, authorization),
            TursoClientSyncRuntimeCommand::Close(session_id) => application.close(&session_id),
        };
        if let Err(error) = result {
            log::error!("[semantic-sync] lifecycle operation failed: {error:#}");
        }
    }
}

struct TursoClientSyncSession {
    authorization: AuthorizationPolicy,
    credentials: Option<TursoClientSyncCredentials>,
    client: Option<TursoClientSync>,
    status: SemanticSyncStatus,
}

/// Owns one provisioned/connected sync lifecycle per viewport session.
///
/// The coordinator deliberately requires explicit `provision`, `connect`,
/// `push_snapshot`, `pull_projection`, `update_authorization`, and `close`
/// calls. No background task or implicit bootstrap can replicate data before
/// the application has authorized and opened the session.
///
/// # Lease and Token Rotation Policy (Milestone 24 / R6)
/// - **Policy Change Rotation**: Authorization policy changes trigger immediate
///   revocation of the old per-session database lease, clearing local credentials
///   and transitioning the session to `SemanticSyncPhase::Stale` with detail
///   `"authorization_changed"`. Reconnecting requires an explicit fresh lease
///   under the new server-approved policy.
/// - **Disconnect Rotation**: Disconnecting or invoking `close` revokes the
///   database lease and destroys local credentials and client connections.
///   Subsequent reconnects start a new authenticated session lifecycle that
///   obtains a brand-new lease.
/// - **Reprovisioning**: Once a session enters `Stale` or `Closed`, the coordinator
///   allows explicit reprovisioning to obtain a fresh lease and reset status.
/// - **Server-Owned Token Lifetime**: Token expiration is governed strictly by
///   server configuration (`TURSO_CLIENT_TOKEN_EXPIRATION`). Milestone 24 does
///   not perform silent in-place token refreshes; token expiration requires
///   explicit reprovisioning. Clients never select or negotiate Turso token lifetime.
#[allow(dead_code)]
pub(crate) struct TursoClientSyncCoordinator<P> {
    provisioner: P,
    sessions: HashMap<SessionId, TursoClientSyncSession>,
    updates: VecDeque<TursoClientSyncUpdate>,
}

#[allow(dead_code)]
impl<P> TursoClientSyncCoordinator<P>
where
    P: TursoClientSyncProvisioner,
{
    pub(crate) fn new(provisioner: P) -> Self {
        Self {
            provisioner,
            sessions: HashMap::new(),
            updates: VecDeque::new(),
        }
    }

    /// Requests an isolated database lease from the server-side provider.
    /// Provisioning alone never opens a local Turso connection.
    pub(crate) fn provision(&mut self, request: TursoClientSyncProvisionRequest) -> Result<()> {
        request
            .session_id
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid semantic-sync session: {error}"))?;
        if request.client_name.trim().is_empty() {
            bail!("semantic-sync client name must not be empty");
        }
        request
            .authorization
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid semantic-sync authorization: {error}"))?;
        require_self_render_sync(&request.authorization)?;

        if let Some(existing) = self.sessions.get(&request.session_id)
            && (existing.credentials.is_some() || existing.client.is_some())
        {
            bail!("semantic-sync session is already provisioned");
        }

        let session_id = request.session_id.clone();
        self.sessions.insert(
            session_id.clone(),
            TursoClientSyncSession {
                authorization: request.authorization.clone(),
                credentials: None,
                client: None,
                status: SemanticSyncStatus::disabled(),
            },
        );
        self.set_status(
            &session_id,
            SemanticSyncStatus::phase(SemanticSyncPhase::Provisioning, None),
        );

        let credentials = match self.provisioner.provision(&request) {
            Ok(credentials) => credentials,
            Err(error) => {
                self.set_status(
                    &session_id,
                    SemanticSyncStatus::phase(
                        SemanticSyncPhase::Failed,
                        Some("provision_failed".to_owned()),
                    ),
                );
                return Err(error).context("provisioning client semantic-sync database");
            }
        };
        let session = self
            .sessions
            .get_mut(&session_id)
            .expect("provisioned semantic-sync session must exist");
        session.credentials = Some(credentials);
        self.set_status(
            &session_id,
            SemanticSyncStatus::phase(SemanticSyncPhase::Provisioned, None),
        );
        Ok(())
    }

    /// Opens the local Turso client only after a successful provisioning lease.
    pub(crate) async fn connect(&mut self, session_id: &SessionId) -> Result<()> {
        let credentials = self
            .sessions
            .get(session_id)
            .and_then(|session| session.credentials.as_ref())
            .map(|credentials| {
                TursoClientSyncCredentials::new(
                    credentials.config.clone(),
                    credentials.auth_token.clone(),
                )
            })
            .transpose()?
            .ok_or_else(|| anyhow::anyhow!("semantic-sync session is not provisioned"))?;
        if self
            .sessions
            .get(session_id)
            .and_then(|session| session.client.as_ref())
            .is_some()
        {
            bail!("semantic-sync session is already connected");
        }

        self.set_status(
            session_id,
            SemanticSyncStatus::phase(SemanticSyncPhase::Connecting, None),
        );
        let client = match TursoClientSync::open(&credentials.config, &credentials.auth_token).await
        {
            Ok(client) => client,
            Err(error) => {
                self.set_status(
                    session_id,
                    SemanticSyncStatus::phase(
                        SemanticSyncPhase::Failed,
                        Some("connect_failed".to_owned()),
                    ),
                );
                return Err(error).context("connecting client semantic-sync database");
            }
        };
        self.sessions
            .get_mut(session_id)
            .expect("connected semantic-sync session must exist")
            .client = Some(client);
        self.set_status(
            session_id,
            SemanticSyncStatus::phase(SemanticSyncPhase::Ready, None),
        );
        Ok(())
    }

    /// Authorizes and pushes one complete server snapshot for the session's
    /// current policy. The raw server snapshot never enters the client table.
    pub(crate) async fn push_snapshot(
        &mut self,
        session_id: &SessionId,
        snapshot: &SemanticSnapshot,
    ) -> Result<AuthorizedSemanticSnapshot> {
        let authorization = self
            .sessions
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("unknown semantic-sync session"))?
            .authorization
            .clone();
        let projection = authorize_snapshot(snapshot, &authorization)?;
        let mut client = self.take_client(session_id)?;
        self.set_status(
            session_id,
            SemanticSyncStatus::phase(SemanticSyncPhase::Pushing, None),
        );
        let result = client.push_projection(&projection).await;
        self.restore_client(session_id, client);
        match result {
            Ok(()) => {
                self.set_status(session_id, ready_status(Some(&projection)));
                Ok(projection)
            }
            Err(error) => {
                self.set_status(
                    session_id,
                    SemanticSyncStatus::phase(
                        SemanticSyncPhase::Failed,
                        Some("push_failed".to_owned()),
                    ),
                );
                Err(error).context("pushing client semantic projection")
            }
        }
    }

    /// Pulls one verified authorized projection from the session database.
    pub(crate) async fn pull_projection(
        &mut self,
        session_id: &SessionId,
    ) -> Result<Option<AuthorizedSemanticSnapshot>> {
        let client = self.take_client(session_id)?;
        self.set_status(
            session_id,
            SemanticSyncStatus::phase(SemanticSyncPhase::Pulling, None),
        );
        let result = client.pull_projection().await;
        self.restore_client(session_id, client);
        match result {
            Ok(projection) => {
                self.set_status(session_id, ready_status(projection.as_ref()));
                Ok(projection)
            }
            Err(error) => {
                self.set_status(
                    session_id,
                    SemanticSyncStatus::phase(
                        SemanticSyncPhase::Failed,
                        Some("pull_failed".to_owned()),
                    ),
                );
                Err(error).context("pulling client semantic projection")
            }
        }
    }

    /// Revokes the old lease and invalidates the local client before a policy
    /// change can be observed as usable synchronization state.
    pub(crate) fn update_authorization(
        &mut self,
        session_id: &SessionId,
        authorization: AuthorizationPolicy,
    ) -> Result<()> {
        authorization
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid semantic-sync authorization: {error}"))?;
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("unknown semantic-sync session"))?;
        if session.authorization == authorization {
            return Ok(());
        }
        let had_lease = session.credentials.is_some() || session.client.is_some();
        session.authorization = authorization;
        session.client = None;
        session.credentials = None;
        let revoke_result = had_lease
            .then(|| self.provisioner.revoke(session_id))
            .unwrap_or(Ok(()));
        match revoke_result {
            Ok(()) => {
                self.set_status(
                    session_id,
                    SemanticSyncStatus::phase(
                        SemanticSyncPhase::Stale,
                        Some("authorization_changed".to_owned()),
                    ),
                );
                Ok(())
            }
            Err(error) => {
                self.set_status(
                    session_id,
                    SemanticSyncStatus::phase(
                        SemanticSyncPhase::Failed,
                        Some("revoke_failed".to_owned()),
                    ),
                );
                Err(error).context("revoking stale client semantic-sync database")
            }
        }
    }

    /// Closes the local client and revokes the remote lease. The closed status
    /// is retained in the update queue even though the session is removed.
    pub(crate) fn close(&mut self, session_id: &SessionId) -> Result<()> {
        let Some(session) = self.sessions.remove(session_id) else {
            return Ok(());
        };
        let had_lease = session.credentials.is_some() || session.client.is_some();
        drop(session.client);
        drop(session.credentials);
        let revoke_result = had_lease
            .then(|| self.provisioner.revoke(session_id))
            .unwrap_or(Ok(()));
        match revoke_result {
            Ok(()) => {
                self.enqueue_update(
                    session_id.clone(),
                    SemanticSyncStatus::phase(SemanticSyncPhase::Closed, None),
                );
                Ok(())
            }
            Err(error) => {
                self.enqueue_update(
                    session_id.clone(),
                    SemanticSyncStatus::phase(
                        SemanticSyncPhase::Failed,
                        Some("revoke_failed".to_owned()),
                    ),
                );
                Err(error).context("revoking closed client semantic-sync database")
            }
        }
    }

    pub(crate) fn status(&self, session_id: &SessionId) -> Option<SemanticSyncStatus> {
        self.sessions
            .get(session_id)
            .map(|session| session.status.clone())
    }

    pub(crate) fn drain_updates(&mut self) -> Vec<TursoClientSyncUpdate> {
        self.updates.drain(..).collect()
    }

    fn take_client(&mut self, session_id: &SessionId) -> Result<TursoClientSync> {
        self.sessions
            .get_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("unknown semantic-sync session"))?
            .client
            .take()
            .ok_or_else(|| anyhow::anyhow!("semantic-sync session is not connected"))
    }

    fn restore_client(&mut self, session_id: &SessionId, client: TursoClientSync) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.client = Some(client);
        }
    }

    fn set_status(&mut self, session_id: &SessionId, status: SemanticSyncStatus) {
        let Some(session) = self.sessions.get_mut(session_id) else {
            return;
        };
        if session.status == status {
            return;
        }
        session.status = status.clone();
        self.enqueue_update(session_id.clone(), status);
    }

    fn enqueue_update(&mut self, session_id: SessionId, status: SemanticSyncStatus) {
        if self.updates.len() >= MAX_SYNC_STATUS_UPDATES {
            self.updates.pop_front();
        }
        self.updates
            .push_back(TursoClientSyncUpdate { session_id, status });
    }
}

fn require_self_render_sync(authorization: &AuthorizationPolicy) -> Result<()> {
    if !authorization.allows_self_render_delivery() {
        bail!("semantic-sync requires self-render delivery authorization");
    }
    if !authorization.allows_model_download() {
        bail!("semantic-sync requires model-download authorization");
    }
    Ok(())
}

fn ready_status(projection: Option<&AuthorizedSemanticSnapshot>) -> SemanticSyncStatus {
    projection.map_or_else(
        || SemanticSyncStatus::phase(SemanticSyncPhase::Ready, None),
        |projection| {
            SemanticSyncStatus::ready(
                projection.source_snapshot_id.0.clone(),
                projection.projection_hash.to_hex(),
            )
        },
    )
}

async fn initialize_client_projection_store(connection: &turso::Connection) -> Result<()> {
    connection
        .execute_batch(CLIENT_PROJECTION_SCHEMA)
        .await
        .context("creating authorized client projection table")?;
    Ok(())
}

async fn replace_client_projection(
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

async fn read_client_projection(
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

fn verify_projection_hash(projection: &AuthorizedSemanticSnapshot) -> Result<()> {
    let expected = projection_hash(projection)?;
    if expected != projection.projection_hash {
        bail!(
            "client projection hash mismatch: expected {}, received {}",
            expected,
            projection.projection_hash
        );
    }
    Ok(())
}

/// Projects a complete server snapshot into the authorized client view.
pub(crate) fn authorize_snapshot(
    snapshot: &SemanticSnapshot,
    policy: &AuthorizationPolicy,
) -> Result<AuthorizedSemanticSnapshot> {
    policy
        .validate()
        .map_err(|error| anyhow::anyhow!("invalid client-sync authorization policy: {error}"))?;
    if !policy.allows_self_render_delivery() {
        bail!("client semantic sync requires self-render delivery authorization");
    }
    if !policy.allows_model_download() {
        bail!("client semantic sync requires model-download authorization");
    }
    if matches!(&snapshot.source, SnapshotSource::GitCommit { .. }) && !policy.allows_history() {
        bail!("client semantic sync of committed snapshots requires history authorization");
    }

    let entities = snapshot
        .entities
        .values()
        .map(|entity| {
            let mut geometry = entity.geometry.clone();
            if let Some(geometry_signature) = geometry.as_mut()
                && geometry_signature
                    .render_blob
                    .as_ref()
                    .is_some_and(|blob_id| !policy.allows_runtime_blob(&blob_id.0))
            {
                geometry_signature.render_blob = None;
            }

            let properties = entity
                .properties
                .iter()
                .filter(|property| policy.allows_semantic_property(&property.name))
                .cloned()
                .collect();

            (
                entity.key.clone(),
                AuthorizedEntitySnapshot {
                    key: entity.key.clone(),
                    prim_path: entity.prim_path.clone(),
                    identity_source: entity.identity_source,
                    semantic: entity.semantic.clone(),
                    transform: entity.transform.clone(),
                    geometry,
                    properties,
                    source_metadata_hash: entity.metadata_hash,
                    source_full_hash: entity.full_hash,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut projected = AuthorizedSemanticSnapshot {
        source_snapshot_id: snapshot.snapshot_id.clone(),
        source: snapshot.source.clone(),
        config_hash: snapshot.config_hash,
        projection_hash: HashDigest::new([0; HashDigest::BYTE_LEN]),
        entities,
    };
    projected.projection_hash = projection_hash(&projected)?;
    Ok(projected)
}

fn projection_hash(snapshot: &AuthorizedSemanticSnapshot) -> Result<HashDigest> {
    let mut canonical = snapshot.clone();
    canonical.projection_hash = HashDigest::new([0; HashDigest::BYTE_LEN]);
    let bytes = serde_json::to_vec(&canonical).context("serializing semantic client projection")?;
    Ok(HashDigest::new(*blake3::hash(&bytes).as_bytes()))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use super::*;
    use usd_model::{
        BlobId, Bounds3, CanonicalValue, EntitySnapshot, GeometrySignature, QuantizedPoint3,
    };
    use viewport_protocol::{
        DeliveryMode, HistoryPermission, ModelDownloadPermission, RuntimeProfile,
        SemanticPropertyScope,
    };

    const MESH_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn digest(value: u8) -> HashDigest {
        HashDigest::new([value; HashDigest::BYTE_LEN])
    }

    fn snapshot() -> SemanticSnapshot {
        let key = EntityKey::from("entity-1");
        SemanticSnapshot {
            snapshot_id: SnapshotId("working-7".to_owned()),
            source: SnapshotSource::Working {
                session: "editor".to_owned(),
                live_revision: 7,
            },
            config_hash: digest(1),
            entities: HashMap::from([(
                key.clone(),
                EntitySnapshot {
                    key,
                    prim_path: "/Root/Asset".to_owned(),
                    identity_source: IdentitySource::PrimPath,
                    semantic: SemanticInfo {
                        category: Some("asset".to_owned()),
                        family: None,
                        type_name: Some("Mesh".to_owned()),
                        type_id: None,
                        display_name: Some("Asset".to_owned()),
                    },
                    transform: TransformSignature {
                        translation_mm: [0, 0, 0],
                        rotation_quantized: [0, 0, 0, 1],
                        scale_quantized: [1, 1, 1],
                        hash: digest(2),
                    },
                    geometry: Some(GeometrySignature {
                        vertex_count: 3,
                        index_count: 3,
                        local_bounds: Bounds3 {
                            min: [0.0, 0.0, 0.0],
                            max: [1.0, 1.0, 1.0],
                        },
                        local_centroid: QuantizedPoint3([500, 500, 500]),
                        topology_hash: digest(3),
                        shape_hash: digest(4),
                        render_blob: Some(BlobId(MESH_ID.to_owned())),
                    }),
                    properties: vec![SemanticProperty {
                        name: "secret_cost".to_owned(),
                        value: CanonicalValue::Integer(42),
                    }],
                    metadata_hash: digest(5),
                    full_hash: digest(6),
                },
            )]),
        }
    }

    fn policy(scope: SemanticPropertyScope, allow_mesh: bool) -> AuthorizationPolicy {
        AuthorizationPolicy {
            allowed_delivery_modes: vec![DeliveryMode::SelfRender],
            model_download: ModelDownloadPermission::Allowed,
            allowed_blob_ids: allow_mesh
                .then_some(MESH_ID.to_owned())
                .or_else(|| {
                    Some(
                        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                            .to_owned(),
                    )
                })
                .into_iter()
                .collect(),
            semantic_property_scope: scope,
            history: HistoryPermission::ReadOnly,
            runtime_profile: RuntimeProfile::NativeMedium,
        }
    }

    #[test]
    fn projection_filters_properties_and_unauthorized_mesh_ids() {
        let projected =
            authorize_snapshot(&snapshot(), &policy(SemanticPropertyScope::None, false))
                .expect("self-render policy should project");
        let entity = projected.entities.values().next().unwrap();

        assert!(entity.properties.is_empty());
        assert_eq!(
            entity.geometry.as_ref().unwrap().render_blob,
            None,
            "unauthorized blob IDs must not enter client metadata"
        );
        assert_eq!(entity.source_full_hash, digest(6));
        assert_ne!(projected.projection_hash, digest(1));
    }

    #[test]
    fn projection_keeps_explicitly_allowed_values_and_blob_ids() {
        let projected = authorize_snapshot(&snapshot(), &policy(SemanticPropertyScope::All, true))
            .expect("self-render policy should project");
        let entity = projected.entities.values().next().unwrap();

        assert_eq!(entity.properties.len(), 1);
        assert_eq!(
            entity.geometry.as_ref().unwrap().render_blob,
            Some(BlobId(MESH_ID.to_owned()))
        );
    }

    #[test]
    fn projection_requires_self_render_and_download_authorization() {
        let visitor = viewport_protocol::AuthorizationPolicy::default();
        assert!(authorize_snapshot(&snapshot(), &visitor).is_err());
    }

    #[test]
    fn projection_hash_is_stable_for_the_same_authorized_view() {
        let policy = policy(SemanticPropertyScope::None, false);
        let first = authorize_snapshot(&snapshot(), &policy).unwrap();
        let second = authorize_snapshot(&snapshot(), &policy).unwrap();
        assert_eq!(first.projection_hash, second.projection_hash);
    }

    #[test]
    fn client_projection_store_round_trips_one_verified_projection() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");
        runtime.block_on(async {
            let database = turso::Builder::new_local(":memory:")
                .build()
                .await
                .expect("local Turso database should build");
            let mut connection = database
                .connect()
                .expect("local Turso connection should open");
            initialize_client_projection_store(&connection)
                .await
                .expect("client projection schema should apply");

            let projection =
                authorize_snapshot(&snapshot(), &policy(SemanticPropertyScope::None, false))
                    .expect("projection should build");
            replace_client_projection(&mut connection, &projection)
                .await
                .expect("projection should store");
            let loaded = read_client_projection(&connection)
                .await
                .expect("projection should read")
                .expect("projection should exist");
            assert_eq!(loaded, projection);
        });
    }

    #[test]
    fn client_sync_config_rejects_missing_transport_credentials() {
        let config = TursoClientSyncConfig {
            local_path: PathBuf::from("client.db"),
            remote_url: "libsql://client.turso.io".to_owned(),
            client_name: "usd-hub-client".to_owned(),
        };
        assert!(config.validate("").is_err());
        assert!(config.validate("token").is_ok());
    }

    #[derive(Clone, Default)]
    struct RecordingProvisioner {
        provisioned: Arc<Mutex<Vec<SessionId>>>,
        revoked: Arc<Mutex<Vec<SessionId>>>,
        fail_revoke: Arc<Mutex<bool>>,
    }

    impl TursoClientSyncProvisioner for RecordingProvisioner {
        fn provision(
            &self,
            request: &TursoClientSyncProvisionRequest,
        ) -> Result<TursoClientSyncCredentials> {
            self.provisioned
                .lock()
                .expect("provision records should not be poisoned")
                .push(request.session_id.clone());
            TursoClientSyncCredentials::new(
                TursoClientSyncConfig {
                    local_path: PathBuf::from(format!("client-{}.db", request.session_id.0)),
                    remote_url: "libsql://client.turso.io".to_owned(),
                    client_name: request.client_name.clone(),
                },
                "opaque-test-token".to_owned(),
            )
        }

        fn revoke(&self, session_id: &SessionId) -> Result<()> {
            if *self.fail_revoke.lock().expect("fail_revoke lock") {
                bail!("simulated revoke failure");
            }
            self.revoked
                .lock()
                .expect("revoke records should not be poisoned")
                .push(session_id.clone());
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct RecordingCloudAdmin {
        created: Arc<Mutex<Vec<(String, String, String)>>>,
        token_requests: Arc<Mutex<Vec<String>>>,
        deleted: Arc<Mutex<Vec<String>>>,
        fail_token: Arc<Mutex<bool>>,
    }

    impl TursoCloudAdmin for RecordingCloudAdmin {
        fn create_database(
            &self,
            organization_slug: &str,
            database_name: &str,
            group_name: &str,
        ) -> Result<TursoCloudDatabase> {
            self.created
                .lock()
                .expect("cloud admin records should not be poisoned")
                .push((
                    organization_slug.to_owned(),
                    database_name.to_owned(),
                    group_name.to_owned(),
                ));
            Ok(TursoCloudDatabase {
                hostname: format!("{database_name}.turso.io"),
            })
        }

        fn create_database_token(
            &self,
            _organization_slug: &str,
            database_name: &str,
            _expiration: Option<&str>,
        ) -> Result<String> {
            self.token_requests
                .lock()
                .expect("cloud admin records should not be poisoned")
                .push(database_name.to_owned());
            if *self
                .fail_token
                .lock()
                .expect("cloud admin records should not be poisoned")
            {
                bail!("test token issuance failure");
            }
            Ok("database-scoped-token".to_owned())
        }

        fn delete_database(&self, _organization_slug: &str, database_name: &str) -> Result<()> {
            self.deleted
                .lock()
                .expect("cloud admin records should not be poisoned")
                .push(database_name.to_owned());
            Ok(())
        }
    }

    fn cloud_config() -> TursoCloudProvisioningConfig {
        TursoCloudProvisioningConfig {
            organization_slug: "usdhub".to_owned(),
            group_name: "default".to_owned(),
            database_prefix: "usdhub-client".to_owned(),
            local_root: PathBuf::from("client-sync"),
            token_expiration: Some("2h".to_owned()),
        }
    }

    fn cloud_request() -> TursoClientSyncProvisionRequest {
        TursoClientSyncProvisionRequest {
            session_id: SessionId::new("session-cloud"),
            client_name: "native-client".to_owned(),
            authorization: policy(SemanticPropertyScope::None, false),
        }
    }

    #[test]
    fn platform_api_config_keeps_secret_out_of_debug_and_normalizes_base_url() {
        let config = TursoPlatformApiConfig::new(
            "https://api.turso.tech/v1/".to_owned(),
            "usdhub".to_owned(),
            "platform-token".to_owned(),
        )
        .expect("valid Platform API configuration should be accepted");

        assert_eq!(config.api_base_url, "https://api.turso.tech/v1");
        assert_eq!(config.organization_slug, "usdhub");
        assert!(
            TursoPlatformApiConfig::new(
                "ftp://api.turso.tech/v1".to_owned(),
                "usdhub".to_owned(),
                "platform-token".to_owned(),
            )
            .is_err()
        );
    }

    #[test]
    fn platform_api_response_models_decode_documented_shapes() {
        let database: CreateDatabaseResponse = serde_json::from_str(
            r#"{"database":{"DbId":"db-id","Hostname":"db-name.turso.io","Name":"db-name"}}"#,
        )
        .expect("documented database response should decode");
        assert_eq!(database.database.hostname, "db-name.turso.io");

        let token: CreateTokenResponse = serde_json::from_str(r#"{"jwt":"database-token"}"#)
            .expect("documented token response should decode");
        assert_eq!(token.jwt, "database-token");
    }

    #[test]
    fn cloud_provider_provisions_scoped_database_and_revokes_once() {
        let admin = RecordingCloudAdmin::default();
        let created = admin.created.clone();
        let token_requests = admin.token_requests.clone();
        let deleted = admin.deleted.clone();
        let provider = TursoCloudProvisioner::new(admin, cloud_config())
            .expect("cloud provider configuration should validate");
        let request = cloud_request();

        let credentials = provider
            .provision(&request)
            .expect("cloud provider should provision a client database");
        assert_eq!(credentials.auth_token, "database-scoped-token");
        assert!(
            credentials
                .config
                .remote_url
                .starts_with("libsql://usdhub-client-")
        );
        assert!(credentials.config.remote_url.ends_with(".turso.io"));
        assert!(credentials.config.local_path.starts_with("client-sync"));
        assert_eq!(created.lock().unwrap().len(), 1);
        assert_eq!(token_requests.lock().unwrap().len(), 1);

        provider
            .revoke(&request.session_id)
            .expect("cloud provider should revoke the client database");
        provider
            .revoke(&request.session_id)
            .expect("revoke should be idempotent after the lease is gone");
        assert_eq!(deleted.lock().unwrap().len(), 1);
    }

    #[test]
    fn cloud_provider_deletes_database_when_token_issuance_fails() {
        let admin = RecordingCloudAdmin::default();
        *admin.fail_token.lock().unwrap() = true;
        let deleted = admin.deleted.clone();
        let provider = TursoCloudProvisioner::new(admin, cloud_config())
            .expect("cloud provider configuration should validate");

        let error = match provider.provision(&cloud_request()) {
            Ok(_) => panic!("token failure should fail provisioning"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("test token issuance failure"));
        assert_eq!(deleted.lock().unwrap().len(), 1);
    }

    #[test]
    fn coordinator_requires_self_render_before_provisioning() {
        let provisioner = RecordingProvisioner::default();
        let records = provisioner.provisioned.clone();
        let mut coordinator = TursoClientSyncCoordinator::new(provisioner);
        let session_id = SessionId::new("session-stream-only");
        let error = coordinator
            .provision(TursoClientSyncProvisionRequest {
                session_id: session_id.clone(),
                client_name: "client".to_owned(),
                authorization: AuthorizationPolicy::default(),
            })
            .expect_err("stream-only sessions must not receive semantic sync");

        assert!(error.to_string().contains("self-render"));
        assert!(records.lock().unwrap().is_empty());
        assert!(coordinator.status(&session_id).is_none());
    }

    #[test]
    fn coordinator_tracks_provisioning_and_revokes_once_on_policy_change() {
        let provisioner = RecordingProvisioner::default();
        let revoked = provisioner.revoked.clone();
        let mut coordinator = TursoClientSyncCoordinator::new(provisioner);
        let session_id = SessionId::new("session-self-render");
        coordinator
            .provision(TursoClientSyncProvisionRequest {
                session_id: session_id.clone(),
                client_name: "client".to_owned(),
                authorization: policy(SemanticPropertyScope::None, false),
            })
            .expect("self-render session should provision");

        assert_eq!(
            coordinator.status(&session_id).unwrap().phase,
            SemanticSyncPhase::Provisioned
        );
        let updates = coordinator.drain_updates();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].status.phase, SemanticSyncPhase::Provisioning);
        assert_eq!(updates[1].status.phase, SemanticSyncPhase::Provisioned);

        coordinator
            .update_authorization(&session_id, AuthorizationPolicy::default())
            .expect("policy change should revoke the old lease");
        assert_eq!(
            coordinator.status(&session_id).unwrap().phase,
            SemanticSyncPhase::Stale
        );
        assert_eq!(revoked.lock().unwrap().as_slice(), &[session_id.clone()]);

        coordinator
            .close(&session_id)
            .expect("closing a stale session should not revoke twice");
        assert_eq!(revoked.lock().unwrap().as_slice(), &[session_id]);
        assert_eq!(
            coordinator.drain_updates().last().unwrap().status.phase,
            SemanticSyncPhase::Closed
        );
    }

    #[test]
    fn runtime_mailbox_enforces_capacity_and_prioritizes_controls() {
        let mailbox = RuntimeMailbox::new();
        let session_a = SessionId::new("session-a");
        let session_b = SessionId::new("session-b");

        // Fill normal capacity
        for i in 0..MAX_PENDING_SYNC_RUNTIME_COMMANDS {
            let cmd = TursoClientSyncRuntimeCommand::Connect(SessionId::new(format!("s-{i}")));
            mailbox.submit(cmd).expect("should submit up to capacity");
        }

        // Next normal command fails with QueueFull without blocking
        let overflow = TursoClientSyncRuntimeCommand::Connect(SessionId::new("s-overflow"));
        assert_eq!(
            mailbox.submit(overflow),
            Err(TursoClientSyncRuntimeSubmitError::QueueFull)
        );

        // Control command for session A is accepted despite full normal queue
        let auth_a = TursoClientSyncRuntimeCommand::UpdateAuthorization {
            session_id: session_a.clone(),
            authorization: AuthorizationPolicy::default(),
        };
        mailbox
            .submit(auth_a.clone())
            .expect("control command must be accepted when normal is full");

        // Close for session B is accepted despite full normal queue
        let close_b = TursoClientSyncRuntimeCommand::Close(session_b.clone());
        mailbox
            .submit(close_b.clone())
            .expect("Close control must be accepted when normal is full");

        // First pop must be a control command
        let first_pop = mailbox.pop().expect("pop should return control");
        assert!(first_pop.is_control());
    }

    #[test]
    fn runtime_mailbox_control_precedence_and_purges_same_session() {
        let mailbox = RuntimeMailbox::new();
        let session_a = SessionId::new("session-a");
        let session_b = SessionId::new("session-b");

        // Queue normal commands for session A and B
        let prov_a = TursoClientSyncRuntimeCommand::Provision(TursoClientSyncProvisionRequest {
            session_id: session_a.clone(),
            client_name: "client-a".to_owned(),
            authorization: policy(SemanticPropertyScope::None, false),
        });
        let conn_a = TursoClientSyncRuntimeCommand::Connect(session_a.clone());
        let conn_b = TursoClientSyncRuntimeCommand::Connect(session_b.clone());

        mailbox.submit(prov_a).unwrap();
        mailbox.submit(conn_a).unwrap();
        mailbox.submit(conn_b.clone()).unwrap();

        // Submit UpdateAuthorization for A -> purges normal A work (prov_a, conn_a)
        let auth_a = TursoClientSyncRuntimeCommand::UpdateAuthorization {
            session_id: session_a.clone(),
            authorization: AuthorizationPolicy::default(),
        };
        mailbox.submit(auth_a).unwrap();

        // Submit Close for A -> supersedes UpdateAuthorization for A
        let close_a = TursoClientSyncRuntimeCommand::Close(session_a.clone());
        mailbox.submit(close_a.clone()).unwrap();

        // Subsequent UpdateAuthorization for A cannot supersede Close
        let auth_a2 = TursoClientSyncRuntimeCommand::UpdateAuthorization {
            session_id: session_a.clone(),
            authorization: AuthorizationPolicy::default(),
        };
        mailbox.submit(auth_a2).unwrap();

        // Pop 1: Close for A (control)
        assert_eq!(mailbox.pop(), Some(close_a));
        // Pop 2: Connect for B (normal B was NOT purged)
        assert_eq!(mailbox.pop(), Some(conn_b));
    }

    #[test]
    fn runtime_mailbox_close_unblocks_worker_and_returns_none() {
        let mailbox = RuntimeMailbox::new();
        let worker_mailbox = mailbox.clone();

        let handle = std::thread::spawn(move || {
            let mut received = Vec::new();
            while let Some(cmd) = worker_mailbox.pop() {
                received.push(cmd);
            }
            received
        });

        mailbox
            .submit(TursoClientSyncRuntimeCommand::Connect(SessionId::new(
                "s-1",
            )))
            .unwrap();

        // Give thread a moment to pop
        std::thread::sleep(std::time::Duration::from_millis(20));

        // Close mailbox
        mailbox.close();

        let received = handle.join().expect("worker thread should join cleanly");
        assert_eq!(received.len(), 1);
        assert_eq!(
            received[0],
            TursoClientSyncRuntimeCommand::Connect(SessionId::new("s-1"))
        );

        // Subsequent submit fails with WorkerUnavailable
        assert_eq!(
            mailbox.submit(TursoClientSyncRuntimeCommand::Connect(SessionId::new(
                "s-2"
            ))),
            Err(TursoClientSyncRuntimeSubmitError::WorkerUnavailable)
        );
    }

    #[test]
    fn runtime_mailbox_worker_exit_marks_mailbox_closed_and_rejects_submits() {
        let mailbox = RuntimeMailbox::new();
        let worker_mailbox = mailbox.clone();

        let handle = std::thread::spawn(move || {
            let _worker_guard = RuntimeMailboxWorkerGuard {
                mailbox: worker_mailbox,
            };
            // Simulate early worker exit (e.g. startup error or crash)
        });

        handle.join().expect("worker thread should join cleanly");

        // Normal submit returns WorkerUnavailable
        let normal_cmd = TursoClientSyncRuntimeCommand::Connect(SessionId::new("s-1"));
        assert_eq!(
            mailbox.submit(normal_cmd),
            Err(TursoClientSyncRuntimeSubmitError::WorkerUnavailable)
        );

        // Control submit also returns WorkerUnavailable
        let control_cmd = TursoClientSyncRuntimeCommand::Close(SessionId::new("s-1"));
        assert_eq!(
            mailbox.submit(control_cmd),
            Err(TursoClientSyncRuntimeSubmitError::WorkerUnavailable)
        );
    }

    #[test]
    fn runtime_drop_closes_mailbox_and_wakes_blocked_worker() {
        let mailbox = RuntimeMailbox::new();
        let worker_mailbox = mailbox.clone();
        let runtime = TursoClientSyncRuntime { mailbox };

        let handle = std::thread::spawn(move || {
            let worker_guard = RuntimeMailboxWorkerGuard {
                mailbox: worker_mailbox,
            };
            let mut received = Vec::new();
            while let Some(cmd) = worker_guard.mailbox.pop() {
                received.push(cmd);
            }
            received
        });

        runtime
            .submit(TursoClientSyncRuntimeCommand::Connect(SessionId::new(
                "s-1",
            )))
            .unwrap();

        // Allow worker to pop and block on next pop()
        std::thread::sleep(std::time::Duration::from_millis(20));

        // Dropping the only runtime handle must close mailbox and wake the worker
        drop(runtime);

        let received = handle
            .join()
            .expect("worker thread should join cleanly without leaking");
        assert_eq!(received.len(), 1);
        assert_eq!(
            received[0],
            TursoClientSyncRuntimeCommand::Connect(SessionId::new("s-1"))
        );
    }

    #[test]
    fn coordinator_reprovisions_fresh_lease_after_stale_or_closed() {
        let provisioner = RecordingProvisioner::default();
        let provisioned = provisioner.provisioned.clone();
        let revoked = provisioner.revoked.clone();
        let mut coordinator = TursoClientSyncCoordinator::new(provisioner);
        let session_id = SessionId::new("session-lifecycle");

        let request = TursoClientSyncProvisionRequest {
            session_id: session_id.clone(),
            client_name: "client".to_owned(),
            authorization: policy(SemanticPropertyScope::None, false),
        };

        // 1. Initial provision
        coordinator.provision(request.clone()).unwrap();
        assert_eq!(
            coordinator.status(&session_id).unwrap().phase,
            SemanticSyncPhase::Provisioned
        );
        assert_eq!(provisioned.lock().unwrap().len(), 1);

        // 2. Invalidate via authorization downgrade -> Stale (revoked once)
        coordinator
            .update_authorization(&session_id, AuthorizationPolicy::default())
            .unwrap();
        assert_eq!(
            coordinator.status(&session_id).unwrap().phase,
            SemanticSyncPhase::Stale
        );
        assert_eq!(revoked.lock().unwrap().len(), 1);

        // 3. Re-provision after Stale -> fresh lease obtained
        coordinator.provision(request.clone()).unwrap();
        assert_eq!(
            coordinator.status(&session_id).unwrap().phase,
            SemanticSyncPhase::Provisioned
        );
        assert_eq!(provisioned.lock().unwrap().len(), 2);

        // 4. Close session -> Closed (revoked once more)
        coordinator.close(&session_id).unwrap();
        assert_eq!(revoked.lock().unwrap().len(), 2);
        assert_eq!(
            coordinator.drain_updates().last().unwrap().status.phase,
            SemanticSyncPhase::Closed
        );

        // 5. Re-provision new lifecycle under same session ID -> fresh lease
        coordinator.provision(request).unwrap();
        assert_eq!(
            coordinator.status(&session_id).unwrap().phase,
            SemanticSyncPhase::Provisioned
        );
        assert_eq!(provisioned.lock().unwrap().len(), 3);
    }

    #[test]
    fn coordinator_revoke_failure_emits_failed_status_with_detail() {
        let provisioner = RecordingProvisioner::default();
        *provisioner.fail_revoke.lock().unwrap() = true;
        let mut coordinator = TursoClientSyncCoordinator::new(provisioner);
        let session_id = SessionId::new("session-revoke-fail");

        coordinator
            .provision(TursoClientSyncProvisionRequest {
                session_id: session_id.clone(),
                client_name: "client".to_owned(),
                authorization: policy(SemanticPropertyScope::None, false),
            })
            .unwrap();

        let result = coordinator.update_authorization(&session_id, AuthorizationPolicy::default());
        assert!(result.is_err());
        let status = coordinator.status(&session_id).unwrap();
        assert_eq!(status.phase, SemanticSyncPhase::Failed);
        assert_eq!(status.detail.as_deref(), Some("revoke_failed"));
    }

    #[test]
    fn runtime_mailbox_closed_session_rejects_subsequent_normal_commands() {
        let mailbox = RuntimeMailbox::new();
        let session_a = SessionId::new("session-a");
        let session_b = SessionId::new("session-b");

        // Submit Close(A)
        mailbox
            .submit(TursoClientSyncRuntimeCommand::Close(session_a.clone()))
            .expect("Close(A) must be accepted");

        // 1. Later Provision(A) rejected with SessionClosed
        let prov_a = TursoClientSyncRuntimeCommand::Provision(TursoClientSyncProvisionRequest {
            session_id: session_a.clone(),
            client_name: "client-a".to_owned(),
            authorization: policy(SemanticPropertyScope::None, false),
        });
        assert_eq!(
            mailbox.submit(prov_a.clone()),
            Err(TursoClientSyncRuntimeSubmitError::SessionClosed)
        );

        // 2. Pop Close(A) -> later Provision(A) still rejected with SessionClosed
        let popped_close = mailbox.pop();
        assert!(popped_close.is_some());
        assert_eq!(
            mailbox.submit(prov_a),
            Err(TursoClientSyncRuntimeSubmitError::SessionClosed)
        );

        // 3. Other operations (Connect, Push, Pull) for session A also rejected
        assert_eq!(
            mailbox.submit(TursoClientSyncRuntimeCommand::Connect(session_a.clone())),
            Err(TursoClientSyncRuntimeSubmitError::SessionClosed)
        );
        assert_eq!(
            mailbox.submit(TursoClientSyncRuntimeCommand::PullProjection(
                session_a.clone()
            )),
            Err(TursoClientSyncRuntimeSubmitError::SessionClosed)
        );

        // 4. Normal work for session B is still accepted
        let prov_b = TursoClientSyncRuntimeCommand::Provision(TursoClientSyncProvisionRequest {
            session_id: session_b.clone(),
            client_name: "client-b".to_owned(),
            authorization: policy(SemanticPropertyScope::None, false),
        });
        assert_eq!(mailbox.submit(prov_b), Ok(()));
    }

    #[test]
    fn runtime_mailbox_update_authorization_allows_later_normal_commands() {
        let mailbox = RuntimeMailbox::new();
        let session_a = SessionId::new("session-a");

        let auth_a = TursoClientSyncRuntimeCommand::UpdateAuthorization {
            session_id: session_a.clone(),
            authorization: AuthorizationPolicy::default(),
        };
        mailbox
            .submit(auth_a)
            .expect("UpdateAuthorization(A) must be accepted");

        // Later normal command for A is allowed (not closed)
        let prov_a = TursoClientSyncRuntimeCommand::Provision(TursoClientSyncProvisionRequest {
            session_id: session_a.clone(),
            client_name: "client-a".to_owned(),
            authorization: policy(SemanticPropertyScope::None, false),
        });
        assert_eq!(mailbox.submit(prov_a), Ok(()));
    }
}
