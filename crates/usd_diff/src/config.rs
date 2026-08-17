//! Configuration for pure snapshot comparison.

/// Comparison options that do not change semantic classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiffConfig {
    /// Whether to materialize property-level metadata changes when the
    /// metadata hash differs.
    pub collect_metadata_changes: bool,
}

impl Default for DiffConfig {
    fn default() -> Self {
        Self {
            collect_metadata_changes: true,
        }
    }
}
