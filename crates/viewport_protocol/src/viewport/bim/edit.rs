//! Authoritative BIM value-edit intent and outcomes.

use serde::{Deserialize, Serialize};
use usd_model::{CanonicalValue, UnitId};

use super::super::constants::MAX_EDITOR_TEXT_BYTES;
use super::super::editor::EditorValue;
use super::super::read_models::SceneAnchor;
use super::constants::{MAX_BIM_FIELD_KEY_BYTES, MAX_BIM_SELECTION_TARGETS};
use crate::ProtocolValidationError;

/// One compare-and-set value edit against one stable scene target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BimPropertyMutation {
    pub target: SceneAnchor,
    pub property: String,
    pub value: EditorValue,
    pub input_unit: Option<UnitId>,
    pub expected_old_value: CanonicalValue,
}

impl BimPropertyMutation {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        self.target.validate()?;
        if self.property.trim().is_empty()
            || self.property.len() > MAX_BIM_FIELD_KEY_BYTES
            || self.property.contains('\0')
        {
            return Err(ProtocolValidationError::InvalidInput {
                field: "bim.edit.property",
            });
        }
        if serde_json::to_vec(&self.value)
            .map(|bytes| bytes.len() > MAX_EDITOR_TEXT_BYTES)
            .unwrap_or(true)
        {
            return Err(ProtocolValidationError::InvalidInput {
                field: "bim.edit.value",
            });
        }
        if serde_json::to_vec(&self.expected_old_value)
            .map(|bytes| bytes.len() > MAX_EDITOR_TEXT_BYTES)
            .unwrap_or(true)
        {
            return Err(ProtocolValidationError::InvalidInput {
                field: "bim.edit.expected_old_value",
            });
        }
        if self
            .input_unit
            .as_ref()
            .is_some_and(|unit| unit.as_str().trim().is_empty() || unit.as_str().contains('\0'))
        {
            return Err(ProtocolValidationError::InvalidInput {
                field: "bim.edit.input_unit",
            });
        }
        Ok(())
    }
}

/// An authoritative result for a compare-and-set mutation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BimPropertyEditOutcome {
    pub target: SceneAnchor,
    pub property: String,
    pub status: BimPropertyEditStatus,
    pub old_value: Option<CanonicalValue>,
    pub new_value: Option<CanonicalValue>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BimPropertyEditStatus {
    Applied,
    Rejected,
}

pub fn validate_bim_mutation_batch(
    mutations: &[BimPropertyMutation],
) -> Result<(), ProtocolValidationError> {
    if mutations.is_empty() || mutations.len() > MAX_BIM_SELECTION_TARGETS {
        return Err(ProtocolValidationError::InvalidInput {
            field: "bim.edit.mutations",
        });
    }
    let property = mutations[0].property.as_str();
    for mutation in mutations {
        mutation.validate()?;
        if mutation.property != property {
            return Err(ProtocolValidationError::InvalidInput {
                field: "bim.edit.mutations.property",
            });
        }
    }
    Ok(())
}
