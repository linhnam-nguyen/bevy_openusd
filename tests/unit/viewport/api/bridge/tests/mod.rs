// Test submodules for src/viewport/api/bridge.
// Split by concern to stay within the 400-line per-file budget.

pub(in crate::viewport) use super::query::refresh_active_hierarchy_projection;

#[path = "bim/bim_edit_tests.rs"]
mod bim_edit_test;
#[path = "bim/bim_properties_lifecycle_tests.rs"]
mod bim_properties_lifecycle_test;
#[path = "bim/bim_properties_tests.rs"]
mod bim_properties_test;
#[path = "commands_tests.rs"]
mod commands_test;
#[path = "editor/edit_rejection_tests.rs"]
mod edit_rejection_tests;
#[path = "query/hierarchy_search_tests.rs"]
mod hierarchy_search_test;
#[path = "query/hierarchy_visibility_tests.rs"]
mod hierarchy_visibility_test;
#[path = "editor/mutations_tests.rs"]
mod mutations_test;
#[path = "settings/renderer_commands_tests.rs"]
mod renderer_commands_test;
#[path = "editor/save_stage_tests.rs"]
mod save_stage_test;
#[path = "selection/selection_boundedness_tests.rs"]
mod selection_boundedness_test;
#[path = "selection/selection_commands_tests.rs"]
mod selection_commands_test;
#[path = "selection/selection_performance_tests.rs"]
mod selection_performance_test;
#[path = "selection/selection_profile_tests.rs"]
mod selection_profile_test;
#[path = "editor/semantic_edit_integration_support.rs"]
mod semantic_edit_integration_support;
#[path = "editor/semantic_edit_integration_tests.rs"]
mod semantic_edit_integration_tests;
#[path = "query/semantic_search_consistency_tests.rs"]
mod semantic_search_consistency_tests;
#[path = "support.rs"]
mod support;
#[path = "settings/viewer_settings_acceptance_tests.rs"]
mod viewer_settings_acceptance_test;
#[path = "settings/viewer_settings_commands_tests.rs"]
mod viewer_settings_commands_test;
#[path = "settings/viewer_settings_gizmo_size_tests.rs"]
mod viewer_settings_gizmo_size_test;
