use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};

pub(crate) mod cache_presentation;
pub(crate) mod projection_readback;

/// Offscreen target resource holding the GPU image handle.
#[derive(Resource, Clone, Debug)]
pub struct OffscreenTarget {
    pub image_handle: Handle<Image>,
    pub width: u32,
    pub height: u32,
    pub generation: u64,
}

/// Marker for the synthetic window used only to feed remote gizmo hover and
/// cursor coordinates. Native camera input must ignore this entity.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HeadlessInputWindow;

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
