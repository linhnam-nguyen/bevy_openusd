//! OR8 M2 deterministic Projects regression harness.
//!
//! The harness is test-only by design. It enters through the same
//! ProjectApplicationService methods used by the application and keeps all
//! generated state below the user-designated OR8/M2 TestSpaces directory.

mod activation;
mod artifacts;
mod assets;
mod composition;
mod fixture;
mod rng;

#[cfg(test)]
mod c1_tests;
#[cfg(test)]
mod c2_tests;
#[cfg(test)]
mod c3_tests;
#[cfg(test)]
mod c4_tests;
