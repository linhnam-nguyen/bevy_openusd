//! In-process implementation of the public viewport bridge contract.
//!
//! This module is decomposed as follows:
//! - [`state`]     — ECS resources and system-set enum
//! - [`plugin`]    — `ViewportBridgePlugin` registration and lifecycle systems
//! - [`query`]    — scene-query dispatch and semantic-search publishing
//! - [`commands`]  — `apply_viewport_commands` system
//! - [`editor`]    — editor command adaptation, authoring policy, mutations, and save
//! - [`tree`]      — `apply_tree_commands` and subtree-geometry helpers
//! - [`helpers`]   — read-model builders and event emitters
//! - [`settings`]  — authoritative protocol settings state
//! - [`convert`]   — `editor_value_to_usd` JSON→USD conversion

mod bim;
mod commands;
pub(crate) mod convert;
mod editor;
mod helpers;
mod plugin;
mod query;
mod settings;
mod state;
mod tree;

#[cfg(test)]
#[path = "../../../../tests/unit/viewport/api/bridge/tests/mod.rs"]
pub(super) mod tests;

pub(crate) use convert::editor_value_to_usd;
pub(crate) use plugin::ViewportBridgePlugin;
pub(in crate::viewport) use settings::ViewerSettingsState;
pub(crate) use state::ViewportBridgeSet;
