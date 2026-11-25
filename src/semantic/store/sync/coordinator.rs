use anyhow::{Context, Result, bail};
use std::collections::{HashMap, VecDeque};
use usd_model::SemanticSnapshot;
use viewport_protocol::{AuthorizationPolicy, SemanticSyncPhase, SemanticSyncStatus, SessionId};

use super::client::TursoClientSync;
use super::client_config::require_self_render_sync;
use super::projection::{AuthorizedSemanticSnapshot, authorize_snapshot};
use super::provisioning::{
    TursoClientSyncCredentials, TursoClientSyncProvisionRequest, TursoClientSyncProvisioner,
};

const MAX_SYNC_STATUS_UPDATES: usize = 256;

/// One status update for the application/session event boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TursoClientSyncUpdate {
    pub session_id: SessionId,
    pub status: SemanticSyncStatus,
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
        let revoke_result = if had_lease {
            self.provisioner.revoke(session_id)
        } else {
            Ok(())
        };
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
        let revoke_result = if had_lease {
            self.provisioner.revoke(session_id)
        } else {
            Ok(())
        };
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
