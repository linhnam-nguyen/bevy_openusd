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
        source_unit: Option<impl Into<String>>,
    ) -> Self {
        Self {
            quantity: QuantitySpecId::new(quantity),
            canonical_unit: UnitId::new(canonical_unit),
            source_unit: source_unit.map(UnitId::new),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_round_trips_without_losing_source_unit() {
        let original = MeasurementMetadata::new("length", "m", Some("ft"));
        let encoded = serde_json::to_string(&original).expect("measurement metadata serializes");
        let decoded: MeasurementMetadata =
            serde_json::from_str(&encoded).expect("measurement metadata deserializes");

        assert_eq!(decoded, original);
        assert_eq!(decoded.quantity.as_str(), "length");
        assert_eq!(decoded.canonical_unit.as_str(), "m");
        assert_eq!(decoded.source_unit.as_ref().map(UnitId::as_str), Some("ft"));
    }

    #[test]
    fn unknown_source_unit_is_distinct_from_a_known_unit() {
        let known = MeasurementMetadata::new("length", "m", Some("m"));
        let unknown = MeasurementMetadata::new("length", "m", None::<String>);

        assert_ne!(known, unknown);
    }
}
