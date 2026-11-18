//! Test-only helpers attached to the owning production module.
use super::*;

pub(crate) fn apply_prepared_activation_for_test(
    world: &mut World,
    request: &ProjectActivationRequest,
    target: Result<Option<crate::project::service::ProjectStageActivationTarget>, String>,
) -> Option<ProjectActivationReply> {
    match apply_prepared_activation(world, request, target) {
        ActivationProgress::Reply(reply) => Some(reply),
        ActivationProgress::Deferred => None,
    }
}

pub(crate) fn continue_deferred_stage_activation_for_test(world: &mut World) {
    continue_deferred_stage_activation(world);
}
