//! Stage-backed BIM property authoring locators and value preparation.

mod locator;
mod value;

#[cfg(test)]
#[path = "authoring_tests.rs"]
mod tests;

pub(crate) use locator::{BimAuthoringError, BimAuthoringLocator, resolve_bim_authoring_locator};
#[cfg(test)]
pub(crate) use locator::{BimEditability, BimNonEditableReason};
pub(crate) use value::{canonical_value_for_comparison, current_bim_value, prepare_bim_value};
