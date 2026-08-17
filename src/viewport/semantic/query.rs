//! Transport-neutral semantic query types.

use usd_model::EntityKey;

/// Query expressed by viewport/application code without exposing SQL.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SemanticQuery {
    pub text: Option<String>,
    pub filters: Vec<SemanticFilter>,
    pub group_by: Vec<GroupField>,
    pub sort: Vec<SortRule>,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SemanticFilter {
    CategoryEquals(String),
    FamilyEquals(String),
    TypeEquals(String),
    PropertyTextEquals {
        name: String,
        value: String,
    },
    PropertyNumberRange {
        name: String,
        min: Option<f64>,
        max: Option<f64>,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum GroupField {
    Category,
    Family,
    TypeName,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SortField {
    DisplayName,
    PrimPath,
    Category,
    Family,
    TypeName,
    TranslationX,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SortRule {
    pub field: SortField,
    pub descending: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SemanticQueryRow {
    pub entity_key: EntityKey,
    pub prim_path: String,
    pub display_name: Option<String>,
    pub category: Option<String>,
    pub family: Option<String>,
    pub type_name: Option<String>,
    pub translation_mm: [i64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SemanticGroup {
    pub field: GroupField,
    pub value: Option<String>,
    pub count: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SemanticQueryResult {
    pub total: u32,
    pub rows: Vec<SemanticQueryRow>,
    pub groups: Vec<SemanticGroup>,
    pub has_more: bool,
}
