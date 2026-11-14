mod bim_classification;
mod bim_properties;
mod chunks;
mod handshake;

use super::session::ApplicationSession;
use crate::application::RenderServerInterface;
use crate::session::SessionAdmission;
use viewport_protocol::{AuthorizationPolicy, ServerCapabilities, SessionId, ViewportReadModel};

impl ApplicationSession {
    pub(crate) fn new(
        session_id: SessionId,
        initial_snapshot: ViewportReadModel,
        interface: RenderServerInterface,
    ) -> Self {
        Self::new_with_capabilities(
            session_id,
            initial_snapshot,
            interface,
            ServerCapabilities::default(),
            AuthorizationPolicy::default(),
            SessionAdmission::default(),
        )
    }
}
