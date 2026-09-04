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
mod c1_tests;
#[cfg(test)]
mod c2_tests;
#[cfg(test)]
mod c3_tests;
#[cfg(test)]
mod c4_tests;
#[cfg(test)]
mod c5_tests;
#[cfg(test)]
mod c6_tests;
#[cfg(test)]
mod c7_tests;
#[cfg(test)]
mod c8_tests;
