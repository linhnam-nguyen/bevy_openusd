//! Stage-backed BIM property authoring locators and value preparation.

mod locator;
mod value;

#[cfg(test)]
#[path = "../../tests/unit/viewport/bim/authoring/tests.rs"]
mod tests;

pub(crate) use locator::{BimAuthoringError, BimAuthoringLocator, resolve_bim_authoring_locator};
pub(crate) use value::{canonical_value_for_comparison, current_bim_value, prepare_bim_value};
