//! Vendored copy of [`transform-gizmo`](https://crates.io/crates/transform-gizmo)
//! v0.9.0 by **Urho Laukkarinen** (`<urho.laukkarinen@gmail.com>`),
//! dual-licensed MIT or Apache-2.0. See `LICENSE-MIT` /
//! `LICENSE-APACHE` next to `bevy_glacial/Cargo.toml`.
//!
//! Core, framework-agnostic transform-gizmo library: takes a
//! [`GizmoConfig`] + [`GizmoInteraction`] each frame, returns a
//! [`GizmoResult`] (translation/rotation/scale deltas) plus tessellated
//! vertices ready to be uploaded by whatever renderer the host wires
//! up. The Bevy adapter sits in [`super`] (one level up).

mod shape;
mod subgizmo;

pub mod bounds;
pub mod config;
pub mod gizmo;
pub mod math;

pub use bounds::{BoundsFace, DEFAULT_BOUNDS_MIN_THICKNESS, resize_transform_face};
pub use config::{GizmoConfig, GizmoDirection, GizmoMode, GizmoOrientation, GizmoVisuals};
pub use gizmo::{Gizmo, GizmoDrawData, GizmoInteraction, GizmoResult};

pub use ecolor::Color32;
pub use emath::Rect;
pub use enumset::{EnumSet, enum_set};
pub use mint;
