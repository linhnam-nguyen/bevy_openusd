//! Transport- and renderer-neutral property values.

/// Canonical value representation used by semantic snapshots.
///
/// This intentionally does not expose `openusd::sdf::Value`; historical
/// snapshots must remain independent from the OpenUSD Rust binding.
#[derive(Clone, Debug, PartialEq)]
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
