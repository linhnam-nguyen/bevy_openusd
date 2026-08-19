use anyhow::{Context, Result};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Condvar, Mutex},
};
use usd_model::SemanticSnapshot;
use viewport_protocol::{AuthorizationPolicy, SemanticSyncStatus, SessionId};

use super::client_config::TursoCloudProvisioningConfig;
use super::coordinator::TursoClientSyncCoordinator;
use super::platform_api::{TursoCloudAdmin, TursoPlatformApi};
use super::projection::AuthorizedSemanticSnapshot;
use super::provisioning::{
    TursoClientSyncProvisionRequest, TursoClientSyncProvisioner, TursoCloudProvisioner,
};

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
    pub(crate) mailbox: RuntimeMailbox,
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

pub(super) struct RuntimeMailboxWorkerGuard {
    pub(super) mailbox: RuntimeMailbox,
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
