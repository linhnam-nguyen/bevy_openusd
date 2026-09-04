use std::collections::HashMap;

use bevy::prelude::Resource;

/// Describes whether a material on a projected prim came from a successful
/// authored USD conversion or from the disposable renderer fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterialProjectionStatus {
    AuthoredConversion,
    Fallback,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct MaterialProjectionProvenance {
    by_prim_path: HashMap<String, MaterialProjectionStatus>,
}

impl MaterialProjectionProvenance {
    pub fn status(&self, prim_path: &str) -> Option<MaterialProjectionStatus> {
        self.by_prim_path.get(prim_path).copied()
    }

    pub(crate) fn mark_authored(&mut self, prim_path: impl Into<String>) {
        self.by_prim_path.insert(
            prim_path.into(),
            MaterialProjectionStatus::AuthoredConversion,
        );
    }

    pub(crate) fn mark_fallback(&mut self, prim_path: impl Into<String>) {
        self.by_prim_path
            .insert(prim_path.into(), MaterialProjectionStatus::Fallback);
    }

    pub fn clear(&mut self) {
        self.by_prim_path.clear();
    }
}
