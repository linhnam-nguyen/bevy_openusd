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

/// Offscreen target resource holding the GPU image handle and rendering resolution.
#[derive(Resource, Clone, Debug)]
pub struct OffscreenTarget {
    pub image_handle: Handle<Image>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// Headless rendering plugin for offscreen Bevy App setup.
pub struct HeadlessRenderPlugin {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl Default for HeadlessRenderPlugin {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 60,
        }
    }
}

impl Plugin for HeadlessRenderPlugin {
    fn build(&self, app: &mut App) {
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();

        let size = Extent3d {
            width: self.width,
            height: self.height,
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
        let image_handle = images.add(image);

        app.insert_resource(OffscreenTarget {
            image_handle,
            width: self.width,
            height: self.height,
            fps: self.fps,
        })
        .add_systems(Startup, setup_offscreen_camera_target);
    }
}

/// Redirects spawned 3D cameras to render into the offscreen GPU image target.
fn setup_offscreen_camera_target(
    target: Res<OffscreenTarget>,
    mut commands: Commands,
    cameras: Query<Entity, With<Camera3d>>,
) {
    let render_target = RenderTarget::Image(target.image_handle.clone().into());
    for camera_entity in &cameras {
        commands.entity(camera_entity).insert(render_target.clone());
    }
}
