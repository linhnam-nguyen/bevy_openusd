//! Test-only helpers attached to the owning production module.
use super::*;

impl ProjectStageActivationRuntime {
    pub(super) fn wait_for_prepared(
        &self,
    ) -> Result<PreparedProjectActivation, std::sync::mpsc::RecvError> {
        // Correctness is synchronized on the worker result itself. The old
        // one-second wall-clock deadline was test-only and could fail under
        // full-suite CPU/filesystem contention even though production uses the
        // non-blocking `take_prepared` path and the worker remained live.
        self.preparation
            .receiver
            .lock()
            .expect("Project activation preparation receiver is not poisoned")
            .recv()
    }
}

pub(crate) fn observe_project_activation_for_test(
    world: &mut bevy::prelude::World,
    session_id: &str,
    command: &project_protocol::ProjectActivationCommand,
) -> bool {
    world
        .resource_mut::<ProjectActivationAuthorityRuntime>()
        .0
        .observe_request(session_id, command)
}
