//! In-process implementation of the public viewport bridge contract.
//!
//! This module is decomposed as follows:
//! - [`state`]     — ECS resources and system-set enum
//! - [`plugin`]    — `ViewportBridgePlugin` registration and lifecycle systems
//! - [`scene_query`] — scene-query dispatch and semantic-search publishing
//! - [`commands`]  — `apply_viewport_commands` system
//! - [`mutations`] — `apply_runtime_mutations` helper
//! - [`tree`]      — `apply_tree_commands` and subtree-geometry helpers
//! - [`helpers`]   — read-model builders and event emitters
//! - [`settings`]  — authoritative protocol settings state
//! - [`convert`]   — `editor_value_to_usd` JSON→USD conversion

mod bim_commands;
mod bim_edit;
mod bim_search;
mod commands;
pub(crate) mod convert;
mod editor_commands;
mod helpers;
mod mutations;
mod plugin;
mod save;
mod scene_query;
mod scene_query_results;
mod settings;
mod state;
mod tree;

#[cfg(test)]
mod tests;

pub(crate) use convert::editor_value_to_usd;
pub(crate) use plugin::ViewportBridgePlugin;
pub(in crate::viewport) use settings::ViewerSettingsState;
pub(crate) use state::ViewportBridgeSet;
