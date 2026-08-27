use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashSet;

use super::identity::SceneAnchor;
use crate::{MAX_SELECTION_TARGETS, ProtocolValidationError};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionReadModel {
    /// The complete authoritative selection set, in deterministic order.
    pub targets: Vec<SceneAnchor>,
    /// The active primary target, which must be a member of [`Self::targets`].
    pub primary: Option<SceneAnchor>,
}

impl SelectionReadModel {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        Self::validate_parts(&self.targets, self.primary.as_ref())
    }

    pub fn validate_parts(
        targets: &[SceneAnchor],
        primary: Option<&SceneAnchor>,
    ) -> Result<(), ProtocolValidationError> {
        if targets.len() > MAX_SELECTION_TARGETS {
            return Err(ProtocolValidationError::InvalidInput {
                field: "selection.targets",
            });
        }

        let mut seen = HashSet::with_capacity(targets.len());
        for target in targets {
            target.validate()?;
            if !seen.insert(target) {
                return Err(ProtocolValidationError::InvalidInput {
                    field: "selection.targets",
                });
            }
        }

        if let Some(primary) = primary {
            primary.validate()?;
            if !targets.contains(primary) {
                return Err(ProtocolValidationError::InvalidInput {
                    field: "selection.primary",
                });
            }
        }
        Ok(())
    }

    pub fn canonicalize(&mut self) -> Result<(), ProtocolValidationError> {
        self.validate()?;
        self.targets.sort_unstable();
        Ok(())
    }

    /// Applies one authoritative server-side selection transaction without
    /// requiring the complete selection set on the wire.
    pub fn apply_delta(
        &mut self,
        added: &[SceneAnchor],
        removed: &[SceneAnchor],
        primary: Option<SceneAnchor>,
        count: u32,
    ) -> Result<(), ProtocolValidationError> {
        validate_delta_targets(added)?;
        validate_delta_targets(removed)?;
        let added_set = added.iter().collect::<HashSet<_>>();
        if removed.iter().any(|target| added_set.contains(target)) {
            return Err(ProtocolValidationError::InvalidInput {
                field: "selection.delta",
            });
        }

        let mut next = self.clone();
        for target in removed {
            if !next.targets.contains(target) {
                return Err(ProtocolValidationError::InvalidInput {
                    field: "selection.removed",
                });
            }
        }
        for target in added {
            if next.targets.contains(target) {
                return Err(ProtocolValidationError::InvalidInput {
                    field: "selection.added",
                });
            }
        }
        next.targets.retain(|target| !removed.contains(target));
        next.targets.extend(added.iter().cloned());
        next.primary = primary;
        next.canonicalize()?;
        if next.targets.len() != count as usize {
            return Err(ProtocolValidationError::InvalidInput {
                field: "selection.count",
            });
        }
        *self = next;
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

fn validate_delta_targets(targets: &[SceneAnchor]) -> Result<(), ProtocolValidationError> {
    if targets.len() > MAX_SELECTION_TARGETS {
        return Err(ProtocolValidationError::InvalidInput {
            field: "selection.delta",
        });
    }
    let mut seen = HashSet::with_capacity(targets.len());
    for target in targets {
        target.validate()?;
        if !seen.insert(target) {
            return Err(ProtocolValidationError::InvalidInput {
                field: "selection.delta",
            });
        }
    }
    Ok(())
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
        self.validate().map_err(serde::ser::Error::custom)?;
        SelectionReadModelWire {
            targets: &self.targets,
            primary: &self.primary,
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
