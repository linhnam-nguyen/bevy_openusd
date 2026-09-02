//! Stage authoring + persistence (RETHINK P5 ops + P6 persistence).
//!
//! The model layer the editor UI drives: namespace edits (define / remove /
//! rename / reparent / move), attribute authoring, and saving. Every edit
//! goes through the live `Stage` (`&self`, interior-mutable) and commits —
//! firing the `StageSink` so [`crate::live`] reprojects the affected
//! entities. The UI panels are a thin presentation layer on top of these;
//! these functions are headless and fully testable without a window.

mod history;
mod ops;

pub use history::{AttributeEdit, EditHistory};
pub use ops::{
    clear_attribute, define_prim, export_stage_string, move_prim, prim_exists, remove_prim,
    rename_prim, reparent_prim, save_stage_as, set_attribute, set_variant,
};

#[cfg(test)]
mod tests;
