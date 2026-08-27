/// Errors raised while constructing or validating pure Project domain values.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProjectDomainError {
    #[error("{kind} identifier must not be nil")]
    NilIdentifier { kind: &'static str },
    #[error("invalid {kind} identifier: {value}")]
    InvalidIdentifier { kind: &'static str, value: String },
    #[error("external model source kind must not be empty")]
    EmptyExternalSourceKind,
}

impl ProjectDomainError {
    pub(crate) fn invalid_identifier(kind: &'static str, value: &str) -> Self {
        Self::InvalidIdentifier {
            kind,
            value: value.to_owned(),
        }
    }
}
