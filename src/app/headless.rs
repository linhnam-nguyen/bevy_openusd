//! Headless Bevy rendering configuration for offscreen GPU rendering.
//!
//! Replaces window-based rendering plugins (`bevy_winit`) with a minimal offscreen
//! render target (`Handle<Image>`) and camera setup, suitable for server-side
//! rendering and video streaming.

use bevy::camera::RenderTarget;
use bevy::prelude::*;

pub(crate) use crate::viewport::rendering::offscreen::{
    HeadlessInputWindow, OffscreenTarget, new_offscreen_image,
};

/// Headless rendering plugin for offscreen Bevy App setup.
pub struct HeadlessRenderPlugin {
    pub width: u32,
    pub height: u32,
}

impl Default for HeadlessRenderPlugin {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
        }
    }
}

impl Plugin for HeadlessRenderPlugin {
    fn build(&self, app: &mut App) {
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let image_handle = images.add(new_offscreen_image(self.width, self.height));

        app.insert_resource(OffscreenTarget {
            image_handle,
            width: self.width,
            height: self.height,
            generation: 0,
        })
        .world_mut()
        .spawn((
            Window {
                resolution: (self.width, self.height).into(),
                focused: false,
                ..default()
            },
            bevy::window::PrimaryWindow,
            HeadlessInputWindow,
        ));

        app.add_systems(Update, setup_offscreen_camera_target);
    }
}

/// Keeps every headless 3D camera bound to the current offscreen image target.
///
/// The Phase 4 implementation used `Added<Camera3d>` plus deferred commands,
/// which was sufficient while the target never changed. Phase 5 can replace
/// the image after the camera is spawned, so the binding must be synchronized
/// directly whenever this system runs.
fn setup_offscreen_camera_target(
    target: Res<OffscreenTarget>,
    mut render_targets: Query<&mut RenderTarget, With<Camera3d>>,
) {
    for mut render_target in &mut render_targets {
        if render_target.as_image() != Some(&target.image_handle) {
            *render_target = RenderTarget::Image(target.image_handle.clone().into());
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/viewport/app/headless_inline_tests.rs"]
mod tests;
