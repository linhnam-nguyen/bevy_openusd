use serde::{Deserialize, Serialize};

/// Project actions currently supported by the application boundary.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectCapabilities {
    pub can_create_scene: bool,
    pub can_import_scene: bool,
    pub can_import_model: bool,
    pub can_switch_branch: bool,
    pub can_commit: bool,
}
