//! Single-pass execution of renderer projection work batches.
//!
//! The direct executor consumes each actual query item exactly once, in
//! iterator order. Callers retain ownership of the projection operations.

#[derive(Clone, Copy)]
pub(crate) enum ProjectionBatch {
    MaterialTransition,
    MaterialRestore,
    MaterialIncremental,
    ShadowTransition,
    #[cfg(feature = "solari")]
    SolariFullScan,
    #[cfg(feature = "solari")]
    SolariIncremental,
}

pub(crate) trait ProjectionWork {
    fn execute<I, F>(&mut self, batch: ProjectionBatch, items: I, apply: F)
    where
        I: IntoIterator,
        F: FnMut(I::Item);
}

pub(crate) struct DirectProjectionWork;

impl ProjectionWork for DirectProjectionWork {
    #[inline]
    fn execute<I, F>(&mut self, _batch: ProjectionBatch, items: I, mut apply: F)
    where
        I: IntoIterator,
        F: FnMut(I::Item),
    {
        for item in items {
            apply(item);
        }
    }
}
