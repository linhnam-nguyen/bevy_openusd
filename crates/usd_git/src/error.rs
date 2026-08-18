use std::{fmt, io, path::PathBuf};

/// Errors returned by the Git history and commit boundary.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Git(String),
    InvalidRevisionSpec(String),
    InvalidPath(PathBuf),
    DestinationNotEmpty(PathBuf),
    UnsupportedEntry { path: PathBuf, kind: String },
    InvalidSourceDirectory(PathBuf),
    UnsupportedSourceEntry { path: PathBuf, kind: String },
    ReadOnly,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn git(error: impl fmt::Display) -> Self {
        Self::Git(error.to_string())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Git(error) => write!(formatter, "Git error: {error}"),
            Self::InvalidRevisionSpec(spec) => {
                write!(formatter, "invalid revision specification: {spec:?}")
            }
            Self::InvalidPath(path) => {
                write!(formatter, "unsafe materialized path: {}", path.display())
            }
            Self::DestinationNotEmpty(path) => {
                write!(
                    formatter,
                    "materialization destination is not empty: {}",
                    path.display()
                )
            }
            Self::UnsupportedEntry { path, kind } => {
                write!(
                    formatter,
                    "unsupported Git tree entry {kind:?} at {}",
                    path.display()
                )
            }
            Self::InvalidSourceDirectory(path) => {
                write!(
                    formatter,
                    "invalid commit source directory: {}",
                    path.display()
                )
            }
            Self::UnsupportedSourceEntry { path, kind } => {
                write!(
                    formatter,
                    "unsupported commit source entry {kind:?} at {}",
                    path.display()
                )
            }
            Self::ReadOnly => formatter.write_str("Git repository backend is read-only"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
