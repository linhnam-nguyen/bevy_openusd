//! Generic Bevy viewport with exactly four responsibility groups (PLAN §3.5.3).

use bevy::prelude::SystemSet;

/// Neutral frame-ordering boundary owned by the generic viewport.
/// API/bridge plugins bind their command-application and event-reduction sets
/// into these neutral phases so generic viewport systems never depend on
/// `crate::api`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ViewportFrameSet {
    ApplyCommands,
    ApplyTreeCommands,
    PublishStageLoadState,
    ReduceEvents,
}

#[cfg(test)]
#[path = "../../tests/unit/viewport/test_harness.rs"]
pub(crate) mod test_harness;

pub(crate) mod navigation;
pub(crate) mod presentation;
pub(crate) mod rendering;
pub(crate) mod stage;
