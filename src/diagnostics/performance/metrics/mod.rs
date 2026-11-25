//! Clustered performance metrics with stable parent-module aliases.
// The parent declares the implementations by path so existing module and test identities remain stable.
pub use super::{aggregate, collector, counters, sample};
