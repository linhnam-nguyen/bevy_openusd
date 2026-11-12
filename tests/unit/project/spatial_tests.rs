use super::*;

#[test]
fn normalization_matrix_covers_axis_and_metric_combinations() {
    let cases = [
        (StageUpAxis::Y, 1.0, Matrix4d::IDENTITY),
        (StageUpAxis::Z, 1.0, Matrix4d::rotation_x(-FRAC_PI_2)),
        (StageUpAxis::Y, 0.01, Matrix4d::scale([0.01; 3])),
        (
            StageUpAxis::Z,
            0.01,
            Matrix4d::scale([0.01; 3]) * Matrix4d::rotation_x(-FRAC_PI_2),
        ),
        (StageUpAxis::Y, 0.001, Matrix4d::scale([0.001; 3])),
    ];
    for (up_axis, meters_per_unit, expected) in cases {
        let actual = source_normalization_transform(&SourceSpatialConvention {
            up_axis,
            meters_per_unit,
            up_axis_was_authored: true,
            meters_per_unit_was_authored: true,
        });
        assert_eq!(actual, expected);
    }
}

#[test]
fn z_up_conversion_maps_asymmetric_basis_to_y_up() {
    let transform = source_normalization_transform(&SourceSpatialConvention {
        up_axis: StageUpAxis::Z,
        meters_per_unit: 1.0,
        up_axis_was_authored: true,
        meters_per_unit_was_authored: true,
    });
    let x = [1.0, 0.0, 0.0, 1.0];
    let y = [0.0, 1.0, 0.0, 1.0];
    let z = [0.0, 0.0, 1.0, 1.0];
    assert_close(transform_point(transform, x), [1.0, 0.0, 0.0]);
    assert_close(transform_point(transform, y), [0.0, 0.0, -1.0]);
    assert_close(transform_point(transform, z), [0.0, 1.0, 0.0]);
}

#[test]
fn asymmetric_fixture_preserves_axis_lengths_after_z_up_centimeter_normalization() {
    let transform = source_normalization_transform(&SourceSpatialConvention {
        up_axis: StageUpAxis::Z,
        meters_per_unit: 0.01,
        up_axis_was_authored: true,
        meters_per_unit_was_authored: true,
    });
    let x_arm = [1.0, 0.0, 0.0, 1.0];
    let y_arm = [0.0, 2.0, 0.0, 1.0];
    let z_arm = [0.0, 0.0, 3.0, 1.0];

    assert_close(transform_point(transform, x_arm), [0.01, 0.0, 0.0]);
    assert_close(transform_point(transform, y_arm), [0.0, 0.0, -0.02]);
    assert_close(transform_point(transform, z_arm), [0.0, 0.03, 0.0]);
}

#[test]
fn canonical_stage_is_explicit_and_native_source_needs_no_second_correction() -> Result<()> {
    let stage = Stage::builder().in_memory("canonical-spatial.usda")?;
    author_canonical_stage(&stage)?;

    assert_eq!(
        inspect_stage(&stage)?,
        SourceSpatialConvention {
            up_axis: StageUpAxis::Y,
            meters_per_unit: 1.0,
            up_axis_was_authored: true,
            meters_per_unit_was_authored: true,
        }
    );
    assert_eq!(
        source_normalization_transform(&SourceSpatialConvention {
            up_axis: USDHUB_UP_AXIS,
            meters_per_unit: USDHUB_METERS_PER_UNIT,
            up_axis_was_authored: true,
            meters_per_unit_was_authored: true,
        }),
        Matrix4d::IDENTITY
    );
    Ok(())
}

fn transform_point(matrix: Matrix4d, point: [f64; 4]) -> [f64; 3] {
    [
        point[0] * matrix.0[0]
            + point[1] * matrix.0[4]
            + point[2] * matrix.0[8]
            + point[3] * matrix.0[12],
        point[0] * matrix.0[1]
            + point[1] * matrix.0[5]
            + point[2] * matrix.0[9]
            + point[3] * matrix.0[13],
        point[0] * matrix.0[2]
            + point[1] * matrix.0[6]
            + point[2] * matrix.0[10]
            + point[3] * matrix.0[14],
    ]
}

fn assert_close(actual: [f64; 3], expected: [f64; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
    }
}
