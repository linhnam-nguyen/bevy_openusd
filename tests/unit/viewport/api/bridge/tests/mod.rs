// Test submodules for src/viewport/api/bridge.
// Split by concern to stay within the 400-line per-file budget.

#[path = "bim_edit_test.rs"]
mod bim_edit_test;
#[path = "bim_properties_lifecycle_test.rs"]
mod bim_properties_lifecycle_test;
#[path = "bim_properties_test.rs"]
mod bim_properties_test;
#[path = "commands_test.rs"]
mod commands_test;
#[path = "hierarchy_search_test.rs"]
mod hierarchy_search_test;
#[path = "hierarchy_visibility_test.rs"]
mod hierarchy_visibility_test;
#[path = "m3_integration_support.rs"]
mod m3_integration_support;
#[path = "m3_integration_test.rs"]
mod m3_integration_test;
#[path = "m8_failure_test.rs"]
mod m8_failure_test;
#[path = "m8_semantic_consistency_test.rs"]
mod m8_semantic_consistency_test;
#[path = "mutations_test.rs"]
mod mutations_test;
#[path = "renderer_commands_test.rs"]
mod renderer_commands_test;
#[path = "save_stage_test.rs"]
mod save_stage_test;
#[path = "selection_boundedness_test.rs"]
mod selection_boundedness_test;
#[path = "selection_commands_test.rs"]
mod selection_commands_test;
#[path = "selection_performance_test.rs"]
mod selection_performance_test;
#[path = "selection_profile_test.rs"]
mod selection_profile_test;
#[path = "support.rs"]
mod support;
#[path = "viewer_settings_acceptance_test.rs"]
mod viewer_settings_acceptance_test;
#[path = "viewer_settings_commands_test.rs"]
mod viewer_settings_commands_test;
#[path = "viewer_settings_gizmo_size_test.rs"]
mod viewer_settings_gizmo_size_test;
