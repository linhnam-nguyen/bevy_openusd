//! Material route: a gprim's bound `UsdShade` Material → [`StandardMaterial`].
//!
//! Runs after the mesh route, replacing the placeholder material the mesh route
//! attaches. Reads the `material:binding` and decodes the bound
//! `UsdPreviewSurface` (and Omni/MaterialX equivalents) via [`crate::read::shade`].
//! Resolves textures from the filesystem or embedded `.usdz` archives into [`Assets<Image>`].

mod archive;
mod builder;
mod material_cache;
mod texture_cache;

pub use builder::MaterialRoute;
pub use material_cache::{MaterialCacheStats, UsdMaterialCache};
pub use texture_cache::{TextureCacheKey, TextureCacheStats, UsdTextureCache};

#[cfg(test)]
mod tests;
