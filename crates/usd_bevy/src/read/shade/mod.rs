//! UsdShade read side: resolve a `Material`'s surface shader and harvest its
//! `UsdPreviewSurface` (and MDL / MaterialX equivalents) inputs into a
//! [`ReadPreviewMaterial`], following connections through `UsdUVTexture` and
//! the MaterialX node graph. Reads through openusd only.

mod channels;
mod resolve;
mod types;

pub use resolve::{material_network_dependencies, read_preview_material};
pub use types::{ReadPreviewMaterial, UvTransform, read_material_binding};
