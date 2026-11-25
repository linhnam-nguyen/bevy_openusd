//! Desktop input adapters for the native viewport.

use bevy::ecs::system::SystemParam;
use bevy::prelude::{NonSend, Res, World};
use usd_bevy::{LiveStage, ProgressiveProjectionState, ProjectionReadiness};

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
}

impl ViewerInteractionGate<'_> {
    pub(crate) fn locked(&self) -> bool {
        interaction_locked(self.stage.as_deref(), self.projection.as_deref())
    }

    pub(crate) fn projection_error(&self) -> Option<&str> {
        let stage = self.stage.as_deref()?;
        let projection = self.projection.as_deref()?;
        (projection.session_id() == Some(stage.session_id()))
            .then(|| projection.last_error())
            .flatten()
    }
}

pub(crate) fn viewer_interaction_locked(world: &World) -> bool {
    interaction_locked(
        world.get_non_send::<LiveStage>(),
        world.get_resource::<ProgressiveProjectionState>(),
    )
}

fn interaction_locked(
    stage: Option<&LiveStage>,
    projection: Option<&ProgressiveProjectionState>,
) -> bool {
    let (Some(stage), Some(projection)) = (stage, projection) else {
        return false;
    };
    projection.session_id() != Some(stage.session_id())
        || (projection.readiness() != ProjectionReadiness::Failed
            && !projection.native_instance_dependencies_ready())
}
