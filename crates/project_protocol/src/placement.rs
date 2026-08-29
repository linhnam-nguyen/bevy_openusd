use std::fmt;

use usd_project::ScenePlacementTransform;

const MATRIX_EPSILON: f64 = 1.0e-9;

/// Source-neutral placement input shared by native and browser callers.
///
/// The matrix form is deliberately raw text at the wire boundary. Parsing and
/// validation happen here, before any Project-domain authoring occurs.
#[derive(Clone, Debug, Default, Eq, serde::Deserialize, PartialEq, serde::Serialize)]
pub enum PlacementSpec {
    #[default]
    Default,
    Matrix(String),
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum PlacementValidationError {
    #[error("placement matrix must contain exactly four non-empty rows")]
    Shape,
    #[error("placement matrix row {row} must contain exactly four values")]
    RowShape { row: usize },
    #[error("placement matrix value {value:?} is not a number")]
    Number { value: String },
    #[error("placement matrix contains a non-finite value")]
    NonFinite,
    #[error("placement matrix must be affine with a final row of 0 0 0 1")]
    NonAffine,
    #[error("placement matrix has a singular 3x3 linear component")]
    Singular,
}

impl PlacementSpec {
    pub fn resolve(&self) -> Result<ScenePlacementTransform, PlacementValidationError> {
        match self {
            Self::Default => Ok(ScenePlacementTransform::IDENTITY),
            Self::Matrix(raw) => parse_matrix(raw),
        }
    }
}

fn parse_matrix(raw: &str) -> Result<ScenePlacementTransform, PlacementValidationError> {
    let rows = raw.lines().collect::<Vec<_>>();
    if rows.len() != 4 || rows.iter().any(|row| row.trim().is_empty()) {
        return Err(PlacementValidationError::Shape);
    }

    let mut values = [0.0; 16];
    for (row_index, row) in rows.iter().enumerate() {
        let columns = row
            .split(|character: char| character == ',' || character.is_whitespace())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if columns.len() != 4 {
            return Err(PlacementValidationError::RowShape { row: row_index + 1 });
        }
        for (column_index, value) in columns.iter().enumerate() {
            let parsed = value
                .parse::<f64>()
                .map_err(|_| PlacementValidationError::Number {
                    value: (*value).to_owned(),
                })?;
            if !parsed.is_finite() {
                return Err(PlacementValidationError::NonFinite);
            }
            values[row_index * 4 + column_index] = parsed;
        }
    }

    if values[3].abs() > MATRIX_EPSILON
        || values[7].abs() > MATRIX_EPSILON
        || values[11].abs() > MATRIX_EPSILON
        || (values[15] - 1.0).abs() > MATRIX_EPSILON
    {
        return Err(PlacementValidationError::NonAffine);
    }
    let determinant = values[0] * (values[5] * values[10] - values[6] * values[9])
        - values[1] * (values[4] * values[10] - values[6] * values[8])
        + values[2] * (values[4] * values[9] - values[5] * values[8]);
    if determinant.abs() <= MATRIX_EPSILON {
        return Err(PlacementValidationError::Singular);
    }
    Ok(ScenePlacementTransform(values))
}

impl fmt::Display for PlacementSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => formatter.write_str("default"),
            Self::Matrix(matrix) => formatter.write_str(matrix),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_placement_is_identity() {
        assert_eq!(
            PlacementSpec::Default.resolve().unwrap(),
            ScenePlacementTransform::IDENTITY
        );
    }

    #[test]
    fn matrix_accepts_whitespace_and_commas() {
        let matrix = PlacementSpec::Matrix("1, 0 0 0\n0 2 0 0\n0 0 -1 0\n3 4 5 1".to_owned())
            .resolve()
            .unwrap();
        assert_eq!(matrix.0[5], 2.0);
        assert_eq!(matrix.0[10], -1.0);
        assert_eq!(matrix.0[12..15], [3.0, 4.0, 5.0]);
    }

    #[test]
    fn matrix_rejects_non_affine_and_singular_values() {
        let non_affine = PlacementSpec::Matrix("1 0 0 0\n0 1 0 0\n0 0 1 0\n0 0 0 2".to_owned());
        assert_eq!(
            non_affine.resolve(),
            Err(PlacementValidationError::NonAffine)
        );

        let singular = PlacementSpec::Matrix("1 0 0 0\n0 0 0 0\n0 0 1 0\n0 0 0 1".to_owned());
        assert_eq!(singular.resolve(), Err(PlacementValidationError::Singular));
    }
}
