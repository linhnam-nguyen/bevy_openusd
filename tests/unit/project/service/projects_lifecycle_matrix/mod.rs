//! OR8 M2 deterministic Projects regression harness.
//!
//! The harness is test-only by design. It enters through the same
//! ProjectApplicationService methods used by the application and keeps all
//! generated state below the user-designated OR8/M2 TestSpaces directory.

mod activation;
mod artifacts;
mod asset_inspection;
mod assets;
mod composition;
mod export;
mod fixture;
mod lifecycle;
mod matrix;
mod matrix_depth;
mod matrix_lifecycle;
mod matrix_persistence;
mod matrix_steps;
mod matrix_verify;
mod registration;
mod rng;
#[cfg(test)]
mod source_equivalence;

#[cfg(test)]
mod asset_inventory_tests;
#[cfg(test)]
mod cloned_registration_tests;
#[cfg(test)]
mod full_lifecycle_matrix_tests;
#[cfg(test)]
mod hierarchy_lookup_tests;
#[cfg(test)]
mod import_link_composition_tests;
#[cfg(test)]
mod scene_activation_tests;
#[cfg(test)]
mod scene_export_roundtrip_tests;
#[cfg(test)]
mod scene_label_identity_tests;
