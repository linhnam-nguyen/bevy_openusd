//! Headless Bevy rendering configuration for offscreen GPU rendering.
//!
//! Replaces window-based rendering plugins (`bevy_winit`) with a minimal offscreen
//! render target (`Handle<Image>`) and camera setup, suitable for server-side
//! rendering and video streaming.

use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};

/// Offscreen target resource holding the GPU image handle.
#[derive(Resource, Clone, Debug)]
pub struct OffscreenTarget {
    pub image_handle: Handle<Image>,
    pub width: u32,
    pub height: u32,
    pub generation: u64,
}

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

pub(crate) fn new_offscreen_image(width: u32, height: u32) -> Image {
    let size = Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };

    let mut image = Image {
        texture_descriptor: TextureDescriptor {
            label: Some("offscreen_bevy_target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        },
        ..default()
    };
    image.resize(size);
    image
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
        .add_systems(Update, setup_offscreen_camera_target);
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
