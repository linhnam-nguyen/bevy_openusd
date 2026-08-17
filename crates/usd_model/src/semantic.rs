//! Common semantic fields used for search and grouping.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SemanticInfo {
    pub category: Option<String>,
    pub family: Option<String>,
    pub type_name: Option<String>,
    pub type_id: Option<String>,
    pub display_name: Option<String>,
}
