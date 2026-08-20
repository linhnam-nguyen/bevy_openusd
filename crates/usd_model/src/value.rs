//! Transport- and renderer-neutral property values.

/// Canonical value representation used by semantic snapshots.
///
/// This intentionally does not expose `openusd::sdf::Value`; historical
/// snapshots must remain independent from the OpenUSD Rust binding.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum CanonicalValue {
    Null,
    Bool(bool),
    Integer(i64),
    Real(f64),
    Text(String),
    TextArray(Vec<String>),
    NumberArray(Vec<f64>),
    Json(String),
}
