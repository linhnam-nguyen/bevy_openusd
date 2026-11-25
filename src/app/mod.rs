//! `bevy_openusd` viewer — primary dogfood binary.
//!
//! Loads a USD file, projects it into a Bevy Scene, and shows the result in
//! a VS-Code-style UI (left activity bar + floating panels). Used
//! throughout plugin development: each milestone gets dropped into this
//! viewer so we can eyeball the projection.
//!
//!   cargo run -- --headless --webrtc path/to/robot.usda
//!
//! Mouse: L+R drag orbit · Middle drag pan · Scroll zoom.
//! Keyboard: T I O ? toggle panels · G X P toggle overlays.

pub(crate) mod cadence;
pub(crate) mod composition;
pub(crate) mod headless;
pub(crate) mod hosts;
pub(crate) mod launch_options;
mod runner;
mod runner_projection;
pub(crate) mod scene;

#[cfg(test)]
#[path = "../../tests/unit/viewport/app/test_harness.rs"]
pub(crate) mod test_harness;
pub(crate) use runner::run;
