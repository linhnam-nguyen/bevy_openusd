use std::path::PathBuf;

/// Input to the Git commit transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitRequest {
    pub message: String,
    pub source_directory: PathBuf,
}

impl CommitRequest {
    pub fn new(message: impl Into<String>, source_directory: impl Into<PathBuf>) -> Self {
        Self {
            message: message.into(),
            source_directory: source_directory.into(),
        }
    }
}
