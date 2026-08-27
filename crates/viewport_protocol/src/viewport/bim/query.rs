//! Typed BIM query intent and classification recipes.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::constants::{
    MAX_BIM_CLASSIFICATION_LEVELS, MAX_BIM_FIELD_KEY_BYTES, MAX_BIM_REGEX_BYTES,
    MAX_BIM_REPLACEMENT_BYTES, MAX_BIM_SEARCH_OFFSET, MAX_BIM_SEARCH_PAGE_SIZE,
};
use crate::ProtocolValidationError;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum BimFieldKey {
    Category,
    Family,
    Type,
    Property(String),
}

impl BimFieldKey {
    pub fn property(name: impl Into<String>) -> Self {
        Self::Property(name.into())
    }

    pub fn stable_key(&self) -> String {
        match self {
            Self::Category => "category".to_owned(),
            Self::Family => "family".to_owned(),
            Self::Type => "type".to_owned(),
            Self::Property(name) => format!("property:{name}"),
        }
    }

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        if let Self::Property(name) = self {
            validate_text("bim.field.property", name, false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClassificationLevel {
    pub id: String,
    pub field: BimFieldKey,
}

impl ClassificationLevel {
    pub fn new(id: impl Into<String>, field: BimFieldKey) -> Self {
        Self {
            id: id.into(),
            field,
        }
    }

    fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_text("bim.classification.level_id", &self.id, false)?;
        self.field.validate()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClassificationRecipe {
    pub levels: Vec<ClassificationLevel>,
}

impl ClassificationRecipe {
    pub fn new(levels: Vec<ClassificationLevel>) -> Self {
        Self { levels }
    }

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        if self.levels.is_empty() || self.levels.len() > MAX_BIM_CLASSIFICATION_LEVELS {
            return Err(ProtocolValidationError::InvalidInput {
                field: "bim.classification.levels",
            });
        }
        let mut ids = HashSet::with_capacity(self.levels.len());
        for level in &self.levels {
            level.validate()?;
            if !ids.insert(&level.id) {
                return Err(ProtocolValidationError::InvalidInput {
                    field: "bim.classification.level_id",
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BimPageRequest {
    pub offset: u32,
    pub limit: u32,
}

impl BimPageRequest {
    pub const fn new(offset: u32, limit: u32) -> Self {
        Self { offset, limit }
    }

    pub fn validate(&self, field: &'static str) -> Result<(), ProtocolValidationError> {
        self.validate_max(field, MAX_BIM_SEARCH_PAGE_SIZE)
    }

    pub fn validate_max(
        &self,
        field: &'static str,
        maximum: u32,
    ) -> Result<(), ProtocolValidationError> {
        if self.offset > MAX_BIM_SEARCH_OFFSET || self.limit == 0 || self.limit > maximum {
            return Err(ProtocolValidationError::InvalidInput { field });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum BimSearchQuery {
    PropertyNameRegex {
        pattern: String,
        page: BimPageRequest,
    },
    PropertyValueRegex {
        pattern: String,
        page: BimPageRequest,
    },
    ObjectPropertyMatch {
        property: String,
        pattern: String,
        page: BimPageRequest,
    },
    ReplacementPreview {
        property: String,
        pattern: String,
        replacement: String,
        page: BimPageRequest,
    },
}

impl BimSearchQuery {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        match self {
            Self::PropertyNameRegex { pattern, page }
            | Self::PropertyValueRegex { pattern, page } => {
                validate_pattern(pattern)?;
                page.validate("bim.search.page")
            }
            Self::ObjectPropertyMatch {
                property,
                pattern,
                page,
            } => {
                validate_text("bim.search.property", property, false)?;
                validate_pattern(pattern)?;
                page.validate("bim.search.page")
            }
            Self::ReplacementPreview {
                property,
                pattern,
                replacement,
                page,
            } => {
                validate_text("bim.search.property", property, false)?;
                validate_pattern(pattern)?;
                if replacement.len() > MAX_BIM_REPLACEMENT_BYTES || replacement.contains('\0') {
                    return Err(ProtocolValidationError::InvalidInput {
                        field: "bim.search.replacement",
                    });
                }
                page.validate("bim.search.page")
            }
        }
    }
}

fn validate_pattern(pattern: &str) -> Result<(), ProtocolValidationError> {
    if pattern.trim().is_empty() || pattern.len() > MAX_BIM_REGEX_BYTES || pattern.contains('\0') {
        return Err(ProtocolValidationError::InvalidInput {
            field: "bim.search.pattern",
        });
    }
    Ok(())
}

fn validate_text(
    field: &'static str,
    value: &str,
    allow_empty: bool,
) -> Result<(), ProtocolValidationError> {
    if (!allow_empty && value.trim().is_empty()) || value.len() > MAX_BIM_FIELD_KEY_BYTES {
        return Err(ProtocolValidationError::InvalidInput { field });
    }
    if value.contains('\0') {
        return Err(ProtocolValidationError::InvalidInput { field });
    }
    Ok(())
}
