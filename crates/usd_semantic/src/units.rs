//! Authoritative semantic unit definitions and conversions.

use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;

use usd_model::{MeasurementMetadata, QuantitySpecId, UnitId};

pub const QUANTITY_LENGTH: &str = "length";
pub const QUANTITY_AREA: &str = "area";
pub const QUANTITY_VOLUME: &str = "volume";
pub const QUANTITY_ANGLE: &str = "angle";
pub const QUANTITY_TEMPERATURE: &str = "temperature";
pub const QUANTITY_POWER: &str = "power";
pub const QUANTITY_PRESSURE: &str = "pressure";
pub const QUANTITY_VELOCITY: &str = "velocity";
pub const QUANTITY_VOLUMETRIC_FLOW: &str = "volumetric_flow";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitDefinition {
    quantity: &'static str,
    canonical_unit: &'static str,
    scale_to_canonical: f64,
    offset_to_canonical: f64,
}

impl UnitDefinition {
    pub const fn new(
        quantity: &'static str,
        canonical_unit: &'static str,
        scale_to_canonical: f64,
        offset_to_canonical: f64,
    ) -> Self {
        Self {
            quantity,
            canonical_unit,
            scale_to_canonical,
            offset_to_canonical,
        }
    }

    pub fn quantity(&self) -> QuantitySpecId {
        QuantitySpecId::new(self.quantity)
    }

    pub fn canonical_unit(&self) -> UnitId {
        UnitId::new(self.canonical_unit)
    }
}

#[derive(Debug)]
pub struct UnitRegistry {
    definitions: HashMap<&'static str, UnitDefinition>,
}

impl UnitRegistry {
    pub fn global() -> &'static Self {
        static REGISTRY: OnceLock<UnitRegistry> = OnceLock::new();
        REGISTRY.get_or_init(Self::new)
    }

    pub fn definition(&self, unit: &UnitId) -> Option<UnitDefinition> {
        self.definitions.get(unit.as_str()).copied()
    }

    pub fn metadata_for(
        &self,
        quantity: &QuantitySpecId,
        source_unit: &UnitId,
    ) -> Result<MeasurementMetadata, UnitConversionError> {
        let definition = self
            .definition(source_unit)
            .ok_or_else(|| UnitConversionError::UnknownUnit(source_unit.as_str().to_owned()))?;
        if definition.quantity != quantity.as_str() {
            return Err(UnitConversionError::QuantityMismatch {
                source: definition.quantity.to_owned(),
                target: quantity.as_str().to_owned(),
            });
        }
        Ok(MeasurementMetadata::new(
            quantity.as_str(),
            definition.canonical_unit,
            Some(source_unit.as_str()),
        ))
    }

    pub fn convert(
        &self,
        value: f64,
        source_unit: &UnitId,
        target_unit: &UnitId,
    ) -> Result<f64, UnitConversionError> {
        if !value.is_finite() {
            return Err(UnitConversionError::NonFiniteValue);
        }
        let source = self
            .definition(source_unit)
            .ok_or_else(|| UnitConversionError::UnknownUnit(source_unit.as_str().to_owned()))?;
        let target = self
            .definition(target_unit)
            .ok_or_else(|| UnitConversionError::UnknownUnit(target_unit.as_str().to_owned()))?;
        if source.quantity != target.quantity {
            return Err(UnitConversionError::QuantityMismatch {
                source: source.quantity.to_owned(),
                target: target.quantity.to_owned(),
            });
        }

        let canonical = value * source.scale_to_canonical + source.offset_to_canonical;
        let converted = (canonical - target.offset_to_canonical) / target.scale_to_canonical;
        converted
            .is_finite()
            .then_some(converted)
            .ok_or(UnitConversionError::NonFiniteValue)
    }

    fn new() -> Self {
        let definitions = [
            length("m", 1.0),
            length("mm", 0.001),
            length("cm", 0.01),
            length("km", 1_000.0),
            length("[in_i]", 0.0254),
            length("[ft_i]", 0.3048),
            area("m2", 1.0),
            area("mm2", 0.000_001),
            area("cm2", 0.0001),
            area("km2", 1_000_000.0),
            area("[in_i]2", 0.00064516),
            area("[ft_i]2", 0.09290304),
            volume("m3", 1.0),
            volume("mm3", 0.000_000_001),
            volume("cm3", 0.000_001),
            volume("L", 0.001),
            volume("[in_i]3", 0.000016387064),
            volume("[ft_i]3", 0.028316846592),
            angle("rad", 1.0),
            angle("deg", std::f64::consts::PI / 180.0),
            temperature("K", 1.0, 0.0),
            temperature("Cel", 1.0, 273.15),
            temperature("[degF]", 5.0 / 9.0, 255.3722222222222),
            power("W", 1.0),
            power("kW", 1_000.0),
            power("[HP]", 745.6998715822702),
            pressure("Pa", 1.0),
            pressure("kPa", 1_000.0),
            pressure("bar", 100_000.0),
            pressure("[psi]", 6_894.757293168),
            velocity("m/s", 1.0),
            velocity("km/h", 1.0 / 3.6),
            velocity("[ft_i]/s", 0.3048),
            velocity("[mi_i]/h", 1609.344 / 3600.0),
            volumetric_flow("m3/s", 1.0),
            volumetric_flow("m3/h", 1.0 / 3_600.0),
            volumetric_flow("L/s", 0.001),
            volumetric_flow("L/min", 0.001 / 60.0),
            volumetric_flow("[ft_i]3/s", 0.028316846592),
        ];
        let mut map = HashMap::with_capacity(definitions.len());
        for (unit, definition) in definitions {
            map.insert(unit, definition);
        }
        Self { definitions: map }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnitConversionError {
    UnknownUnit(String),
    QuantityMismatch { source: String, target: String },
    NonFiniteValue,
}

impl fmt::Display for UnitConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownUnit(unit) => write!(formatter, "unknown unit {unit}"),
            Self::QuantityMismatch { source, target } => {
                write!(formatter, "cannot convert {source} to {target}")
            }
            Self::NonFiniteValue => formatter.write_str("measurement value must be finite"),
        }
    }
}

impl std::error::Error for UnitConversionError {}

fn length(unit: &'static str, scale: f64) -> (&'static str, UnitDefinition) {
    (unit, UnitDefinition::new(QUANTITY_LENGTH, "m", scale, 0.0))
}

fn area(unit: &'static str, scale: f64) -> (&'static str, UnitDefinition) {
    (unit, UnitDefinition::new(QUANTITY_AREA, "m2", scale, 0.0))
}

fn volume(unit: &'static str, scale: f64) -> (&'static str, UnitDefinition) {
    (unit, UnitDefinition::new(QUANTITY_VOLUME, "m3", scale, 0.0))
}

fn angle(unit: &'static str, scale: f64) -> (&'static str, UnitDefinition) {
    (unit, UnitDefinition::new(QUANTITY_ANGLE, "rad", scale, 0.0))
}

fn temperature(unit: &'static str, scale: f64, offset: f64) -> (&'static str, UnitDefinition) {
    (
        unit,
        UnitDefinition::new(QUANTITY_TEMPERATURE, "K", scale, offset),
    )
}

fn power(unit: &'static str, scale: f64) -> (&'static str, UnitDefinition) {
    (unit, UnitDefinition::new(QUANTITY_POWER, "W", scale, 0.0))
}

fn pressure(unit: &'static str, scale: f64) -> (&'static str, UnitDefinition) {
    (
        unit,
        UnitDefinition::new(QUANTITY_PRESSURE, "Pa", scale, 0.0),
    )
}

fn velocity(unit: &'static str, scale: f64) -> (&'static str, UnitDefinition) {
    (
        unit,
        UnitDefinition::new(QUANTITY_VELOCITY, "m/s", scale, 0.0),
    )
}

fn volumetric_flow(unit: &'static str, scale: f64) -> (&'static str, UnitDefinition) {
    (
        unit,
        UnitDefinition::new(QUANTITY_VOLUMETRIC_FLOW, "m3/s", scale, 0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(value: &str) -> UnitId {
        UnitId::new(value)
    }

    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    #[test]
    fn registry_looks_up_each_supported_quantity() {
        let registry = UnitRegistry::global();
        for (quantity, source, canonical) in [
            (QUANTITY_LENGTH, "mm", "m"),
            (QUANTITY_AREA, "[ft_i]2", "m2"),
            (QUANTITY_VOLUME, "L", "m3"),
            (QUANTITY_ANGLE, "deg", "rad"),
            (QUANTITY_TEMPERATURE, "Cel", "K"),
            (QUANTITY_POWER, "kW", "W"),
            (QUANTITY_PRESSURE, "[psi]", "Pa"),
            (QUANTITY_VELOCITY, "km/h", "m/s"),
            (QUANTITY_VOLUMETRIC_FLOW, "L/min", "m3/s"),
        ] {
            let definition = registry
                .definition(&unit(source))
                .expect("supported unit is registered");
            assert_eq!(definition.quantity().as_str(), quantity);
            assert_eq!(definition.canonical_unit().as_str(), canonical);
        }
    }

    #[test]
    fn conversions_cover_scale_and_temperature_offsets() {
        let registry = UnitRegistry::global();
        close(
            registry.convert(1.0, &unit("mm"), &unit("m")).unwrap(),
            0.001,
        );
        close(
            registry
                .convert(1.0, &unit("[ft_i]2"), &unit("m2"))
                .unwrap(),
            0.09290304,
        );
        close(
            registry.convert(1.0, &unit("L"), &unit("m3")).unwrap(),
            0.001,
        );
        close(
            registry.convert(180.0, &unit("deg"), &unit("rad")).unwrap(),
            std::f64::consts::PI,
        );
        close(
            registry.convert(0.0, &unit("Cel"), &unit("K")).unwrap(),
            273.15,
        );
        close(
            registry
                .convert(32.0, &unit("[degF]"), &unit("Cel"))
                .unwrap(),
            0.0,
        );
        close(
            registry.convert(2.0, &unit("kW"), &unit("W")).unwrap(),
            2_000.0,
        );
        close(
            registry.convert(1.0, &unit("bar"), &unit("Pa")).unwrap(),
            100_000.0,
        );
        close(
            registry.convert(36.0, &unit("km/h"), &unit("m/s")).unwrap(),
            10.0,
        );
        close(
            registry
                .convert(60.0, &unit("L/min"), &unit("m3/s"))
                .unwrap(),
            0.001,
        );
    }

    #[test]
    fn metadata_uses_the_registry_canonical_unit() {
        let metadata = UnitRegistry::global()
            .metadata_for(&QuantitySpecId::new(QUANTITY_LENGTH), &unit("[ft_i]"))
            .unwrap();

        assert_eq!(metadata.canonical_unit.as_str(), "m");
        assert_eq!(metadata.source_unit.as_ref().unwrap().as_str(), "[ft_i]");
    }

    #[test]
    fn invalid_conversion_is_explicit() {
        let registry = UnitRegistry::global();
        assert_eq!(
            registry.convert(1.0, &unit("unknown"), &unit("m")),
            Err(UnitConversionError::UnknownUnit("unknown".to_owned()))
        );
        assert_eq!(
            registry.convert(1.0, &unit("m"), &unit("Pa")),
            Err(UnitConversionError::QuantityMismatch {
                source: QUANTITY_LENGTH.to_owned(),
                target: QUANTITY_PRESSURE.to_owned(),
            })
        );
        assert_eq!(
            registry.convert(f64::NAN, &unit("m"), &unit("m")),
            Err(UnitConversionError::NonFiniteValue)
        );
    }
}
