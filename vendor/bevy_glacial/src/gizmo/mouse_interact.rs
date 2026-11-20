use bevy::app::{App, Plugin, Update};
use bevy::ecs::{message::MessageWriter, system::Res};
use bevy::input::{ButtonInput, mouse::MouseButton};

use super::{GizmoDragStarted, GizmoDragging};

pub struct MouseGizmoInteractionPlugin;
impl Plugin for MouseGizmoInteractionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, mouse_interact_gizmo);
    }
}

fn mouse_interact_gizmo(
    mouse: Res<ButtonInput<MouseButton>>,
    mut drag_started: MessageWriter<GizmoDragStarted>,
    mut dragging: MessageWriter<GizmoDragging>,
) {
    if mouse.just_pressed(MouseButton::Left) {
        drag_started.write_default();
    }

    if mouse.pressed(MouseButton::Left) {
        dragging.write_default();
    }
}
