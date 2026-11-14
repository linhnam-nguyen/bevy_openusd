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
