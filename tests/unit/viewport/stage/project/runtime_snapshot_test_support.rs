//! Test-only helpers attached to the owning production module.
use super::*;

pub(crate) fn runtime_snapshot_response_with_claim_hook(
    world: &mut bevy::prelude::World,
    request_id: &str,
    project_id: ProjectId,
    active_scene_id: SceneId,
    queue: &ProjectRuntimeAuthorityQueue,
    project_root: &Path,
    before_claim: &dyn Fn(),
) -> ProjectRuntimeResponse {
    runtime_snapshot_response_inner(
        world,
        request_id,
        project_id,
        active_scene_id,
        queue,
        project_root,
        Some(before_claim),
    )
}
