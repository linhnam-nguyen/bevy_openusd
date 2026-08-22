use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashSet;

use super::identity::SceneAnchor;
use crate::ProtocolValidationError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionReadModel {
    /// The complete authoritative selection set, in deterministic order.
    pub targets: Vec<SceneAnchor>,
    /// The active primary target, which must be a member of [`Self::targets`].
    pub primary: Option<SceneAnchor>,
}

impl SelectionReadModel {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        let mut seen = HashSet::with_capacity(self.targets.len());
        for target in &self.targets {
            target.validate()?;
            if !seen.insert(target) {
                return Err(ProtocolValidationError::InvalidInput {
                    field: "selection.targets",
                });
            }
        }

        if let Some(primary) = &self.primary {
            primary.validate()?;
            if !self.targets.contains(primary) {
                return Err(ProtocolValidationError::InvalidInput {
                    field: "selection.primary",
                });
            }
        }
        Ok(())
    }

    pub fn canonicalize(&mut self) -> Result<(), ProtocolValidationError> {
        self.validate()?;
        self.targets.sort();
        Ok(())
    }

    pub fn from_legacy_target(target: Option<SceneAnchor>) -> Self {
        let Some(target) = target else {
            return Self::default();
        };
        Self {
            targets: vec![target.clone()],
            primary: Some(target),
        }
    }
}

#[derive(Serialize)]
struct SelectionReadModelWire<'a> {
    targets: &'a [SceneAnchor],
    primary: &'a Option<SceneAnchor>,
}

#[derive(Deserialize)]
struct SelectionReadModelInput {
    #[serde(default)]
    targets: Vec<SceneAnchor>,
    #[serde(default)]
    primary: Option<SceneAnchor>,
    #[serde(default)]
    target: Option<SceneAnchor>,
}

impl Serialize for SelectionReadModel {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut canonical = self.clone();
        canonical
            .canonicalize()
            .map_err(serde::ser::Error::custom)?;
        SelectionReadModelWire {
            targets: &canonical.targets,
            primary: &canonical.primary,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SelectionReadModel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = SelectionReadModelInput::deserialize(deserializer)?;
        let has_new_fields = !input.targets.is_empty() || input.primary.is_some();
        if input.target.is_some() && has_new_fields {
            return Err(D::Error::custom(
                "selection cannot contain both legacy target and multi-selection fields",
            ));
        }

        let mut selection = if input.target.is_some() {
            Self::from_legacy_target(input.target)
        } else {
            Self {
                targets: input.targets,
                primary: input.primary,
            }
        };
        selection.canonicalize().map_err(D::Error::custom)?;
        Ok(selection)
    }
}
