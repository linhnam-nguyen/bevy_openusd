//! Source-neutral measurement metadata.

use serde::{Deserialize, Serialize};

/// Stable identifier for a physical quantity such as length or pressure.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct QuantitySpecId(pub String);

impl QuantitySpecId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable, UCUM-compatible unit identifier.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct UnitId(pub String);

impl UnitId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Measurement facts attached to an authored semantic property.
///
/// `canonical_unit` identifies the unit used for normalized comparisons and
/// persistence. `source_unit` is retained when the source export explicitly
/// provides it; it is never inferred from a property name or display label.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MeasurementMetadata {
    pub quantity: QuantitySpecId,
    pub canonical_unit: UnitId,
    pub source_unit: Option<UnitId>,
}

impl MeasurementMetadata {
    pub fn new(
        quantity: impl Into<String>,
        canonical_unit: impl Into<String>,
        source_unit: Option<String>,
    ) -> Self {
        Self {
            quantity: QuantitySpecId::new(quantity),
            canonical_unit: UnitId::new(canonical_unit),
            source_unit: source_unit.map(UnitId::new),
        }
    }
}

#[cfg(test)]
#[path = "../tests/contracts/measurement_tests.rs"]
mod tests;
