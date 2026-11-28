//! Desktop input adapters for the native viewport.

use bevy::ecs::system::SystemParam;
use bevy::prelude::{NonSend, Res, World};
use usd_bevy::{LiveStage, ProgressiveProjectionState, ProjectionReadiness};
use viewport_protocol::InitialGpuProjection;

use crate::viewport::stage::{StageActivationInputLock, StageInfo};

mod headless_gizmo;
pub(crate) mod navigation;

pub(crate) use headless_gizmo::sync_headless_gizmo_input;
pub(crate) use navigation::{
    HostUiInputCapture, ViewportNavigationInput, apply_local_navigation_input,
    reset_navigation_frame, ui_wants_keyboard_input,
    ui_wants_pointer_input,
};

#[derive(SystemParam)]
pub(crate) struct ViewerInteractionGate<'w> {
    stage: Option<NonSend<'w, LiveStage>>,
    projection: Option<Res<'w, ProgressiveProjectionState>>,
    stage_info: Option<Res<'w, StageInfo>>,
    activation: Option<Res<'w, StageActivationInputLock>>,
}

impl ViewerInteractionGate<'_> {
    pub(crate) fn locked(&self) -> bool {
        self.activation.is_some()
            || interaction_locked(self.stage.as_deref(), self.projection.as_deref(), self.stage_info.as_deref())
    }

    pub(crate) fn transition(&self) -> bool {
        self.activation.is_some() || self.stage_info.as_ref()
            .and_then(|info| info.initial_gpu_projection.as_ref())
            .is_some_and(|status| *status != InitialGpuProjection::Complete)
    }

    pub(crate) fn projection_error(&self) -> Option<&str> {
        if let Some(InitialGpuProjection::Failed { message }) = self.stage_info.as_ref()
            .and_then(|info| info.initial_gpu_projection.as_ref())
        {
            return Some(message);
        }
        let stage = self.stage.as_deref()?;
        let projection = self.projection.as_deref()?;
        (projection.session_id() == Some(stage.session_id()))
            .then(|| projection.last_error())
            .flatten()
    }
}

pub(crate) fn viewer_interaction_locked(world: &World) -> bool {
    world.contains_resource::<StageActivationInputLock>() || interaction_locked(
        world.get_non_send::<LiveStage>(),
        world.get_resource::<ProgressiveProjectionState>(),
        world.get_resource::<StageInfo>(),
    )
}

fn interaction_locked(
    stage: Option<&LiveStage>,
    projection: Option<&ProgressiveProjectionState>,
    stage_info: Option<&StageInfo>,
) -> bool {
    if stage_info.and_then(|info| info.initial_gpu_projection.as_ref())
        .is_some_and(|status| *status != InitialGpuProjection::Complete)
    {
        return true;
    }
    let (Some(stage), Some(projection)) = (stage, projection) else {
        return false;
    };
    projection.session_id() != Some(stage.session_id())
        || (projection.readiness() != ProjectionReadiness::Failed
            && !projection.native_instance_dependencies_ready())
}
