use bevy::prelude::*;

/// A deliberately configurable safety default, not a frozen product limit.
pub(in crate::viewport) const DEFAULT_AUTOMATIC_COARSE_RENDERABLE_LIMIT: usize = 2_048;

/// Renderer policy for presenting a logical selection.
///
/// The logical selection remains authoritative in either mode. The automatic
/// limit protects a loaded scene today while leaving final product tuning to a
/// later approved milestone.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::viewport) struct SelectionPresentationPolicy {
    pub(in crate::viewport) coarse: bool,
    pub(in crate::viewport) automatic_renderable_limit: Option<usize>,
}

impl Default for SelectionPresentationPolicy {
    fn default() -> Self {
        Self {
            coarse: false,
            automatic_renderable_limit: Some(DEFAULT_AUTOMATIC_COARSE_RENDERABLE_LIMIT),
        }
    }
}

impl SelectionPresentationPolicy {
    pub(in crate::viewport) fn uses_coarse(&self, renderable_count: usize) -> bool {
        self.coarse
            || self
                .automatic_renderable_limit
                .is_some_and(|limit| renderable_count >= limit)
    }
}

/// Renderer-owned marker for a coarse logical-selection presentation.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::viewport) struct CoarseSelectionPresentation;
