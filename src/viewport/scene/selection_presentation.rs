use bevy::prelude::*;

/// Renderer policy for presenting a logical selection.
///
/// The logical selection remains authoritative in either mode. A renderer or
/// product policy can opt into coarse presentation without this module
/// choosing a final mesh-count threshold prematurely.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::viewport) struct SelectionPresentationPolicy {
    pub(in crate::viewport) coarse: bool,
}

/// Renderer-owned marker for a coarse logical-selection presentation.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::viewport) struct CoarseSelectionPresentation;
