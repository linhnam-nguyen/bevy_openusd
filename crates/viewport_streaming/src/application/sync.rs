use std::collections::HashMap;
use viewport_protocol::{
    AuthorizationPolicy, RuntimeManifest, SemanticSyncOperation, SemanticSyncStatus, SessionId,
    validate_runtime_blob_id,
};

use super::interface::RenderServerInterface;
use super::types::{
    MAX_PENDING_MESSAGES, RenderServerPortError, SemanticSyncRequest, SemanticSyncRequestKind,
};

impl RenderServerInterface {
    pub fn submit_semantic_sync_request(
        &self,
        request: SemanticSyncRequest,
    ) -> Result<(), RenderServerPortError> {
        if request.request_id.trim().is_empty()
            || request.client_name.trim().is_empty()
            || request.session_id.validate().is_err()
            || request.authorization.validate().is_err()
        {
            return Err(RenderServerPortError::InvalidPayload);
        }
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        if pending.closed_sessions.contains(&request.session_id) {
            return Err(RenderServerPortError::SessionClosed);
        }
        if pending.semantic_sync_requests.len() >= MAX_PENDING_MESSAGES {
            return Err(RenderServerPortError::QueueFull);
        }
        pending.semantic_sync_requests.push_back(request);
        Ok(())
    }

    /// Submits an internal server lifecycle/security control request.
    pub(crate) fn submit_semantic_sync_control_request(
        &self,
        request: SemanticSyncRequest,
    ) -> Result<(), RenderServerPortError> {
        if request.request_id.trim().is_empty()
            || request.client_name.trim().is_empty()
            || request.session_id.validate().is_err()
            || request.authorization.validate().is_err()
        {
            return Err(RenderServerPortError::InvalidPayload);
        }

        let is_allowed_control = match &request.kind {
            SemanticSyncRequestKind::AuthorizationChanged => true,
            SemanticSyncRequestKind::Client(SemanticSyncOperation::Close) => true,
            SemanticSyncRequestKind::Client(_) => false,
        };
        if !is_allowed_control {
            return Err(RenderServerPortError::InvalidPayload);
        }

        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;

        let session_id = request.session_id.clone();
        if matches!(
            request.kind,
            SemanticSyncRequestKind::Client(SemanticSyncOperation::Close)
        ) {
            pending.closed_sessions.insert(session_id.clone());
        }

        let should_insert = match pending
            .semantic_sync_control_requests
            .get(&request.session_id)
        {
            Some(existing) => match (&existing.kind, &request.kind) {
                (
                    SemanticSyncRequestKind::Client(SemanticSyncOperation::Close),
                    SemanticSyncRequestKind::AuthorizationChanged,
                ) => false,
                _ => true,
            },
            None => true,
        };

        if should_insert {
            pending
                .semantic_sync_control_requests
                .insert(session_id.clone(), request);
        }

        pending
            .semantic_sync_requests
            .retain(|req| req.session_id != session_id);

        Ok(())
    }

    pub fn pop_semantic_sync_request(&self) -> Option<SemanticSyncRequest> {
        let mut pending = self
            .pending
            .lock()
            .expect("render-server interface queue is not poisoned");
        if let Some(session_id) = pending
            .semantic_sync_control_requests
            .keys()
            .next()
            .cloned()
        {
            return pending.semantic_sync_control_requests.remove(&session_id);
        }
        pending.semantic_sync_requests.pop_front()
    }

    /// Publishes the current server-owned runtime inventory.
    pub fn publish_runtime_manifest(
        &self,
        manifest: RuntimeManifest,
    ) -> Result<(), RenderServerPortError> {
        manifest
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        pending.runtime_manifest = Some(manifest);
        Ok(())
    }

    /// Replaces the server-owned authorization policy.
    pub fn publish_authorization_policy(
        &self,
        authorization: AuthorizationPolicy,
    ) -> Result<(), RenderServerPortError> {
        authorization
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        pending.authorization = authorization;
        Ok(())
    }

    pub fn authorization_policy(&self) -> AuthorizationPolicy {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .authorization
            .clone()
    }

    /// Publishes the newest semantic-sync lifecycle state for one session.
    pub fn publish_semantic_sync_status(
        &self,
        session_id: SessionId,
        status: SemanticSyncStatus,
    ) -> Result<(), RenderServerPortError> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        pending.semantic_sync_statuses.insert(session_id, status);
        Ok(())
    }

    pub fn semantic_sync_status(&self, session_id: &SessionId) -> Option<SemanticSyncStatus> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .semantic_sync_statuses
            .get(session_id)
            .cloned()
    }

    pub fn clear_semantic_sync_status(&self, session_id: &SessionId) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.semantic_sync_statuses.remove(session_id);
        }
    }

    /// Atomically replaces the runtime manifest and verified blob bytes.
    pub fn publish_runtime_delivery(
        &self,
        manifest: RuntimeManifest,
        blobs: Vec<(String, Vec<u8>)>,
    ) -> Result<(), RenderServerPortError> {
        manifest
            .validate()
            .map_err(|_| RenderServerPortError::InvalidPayload)?;
        let mut blob_map = HashMap::with_capacity(blobs.len());
        for (blob_id, bytes) in blobs {
            if validate_runtime_blob_id(&blob_id).is_err()
                || blob_map.insert(blob_id, bytes).is_some()
            {
                return Err(RenderServerPortError::InvalidPayload);
            }
        }
        let references = manifest.references();
        if references.len() != blob_map.len()
            || references.iter().any(|reference| {
                blob_map
                    .get(&reference.blob_id)
                    .is_none_or(|bytes| bytes.len() as u64 != reference.byte_size)
            })
        {
            return Err(RenderServerPortError::InvalidPayload);
        }

        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        pending.runtime_manifest = Some(manifest);
        pending.runtime_blobs = blob_map;
        Ok(())
    }

    pub fn clear_runtime_delivery(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.runtime_manifest = None;
            pending.runtime_blobs.clear();
        }
    }

    /// Publishes verified bytes for one content-addressed runtime object.
    pub fn publish_runtime_blob(
        &self,
        blob_id: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<(), RenderServerPortError> {
        let blob_id = blob_id.into();
        if validate_runtime_blob_id(&blob_id).is_err() {
            return Err(RenderServerPortError::InvalidPayload);
        }
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RenderServerPortError::QueueClosed)?;
        pending.runtime_blobs.insert(blob_id, bytes);
        Ok(())
    }

    pub fn runtime_manifest(&self) -> Option<RuntimeManifest> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .runtime_manifest
            .clone()
    }

    pub fn runtime_blob(&self, blob_id: &str) -> Option<Vec<u8>> {
        self.pending
            .lock()
            .expect("render-server interface queue is not poisoned")
            .runtime_blobs
            .get(blob_id)
            .cloned()
    }
}
