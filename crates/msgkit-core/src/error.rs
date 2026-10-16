use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by msgkit. Individual malformed records are reported as
/// warnings in an [`ImportReport`](crate::ImportReport) instead.
#[derive(Debug, Error)]
pub enum Error {
    /// The caller supplied input the operation cannot work with.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// The input was well-formed JSON/text but not a supported format.
    #[error("parse error: {0}")]
    Parse(String),
    #[error("unknown operation: {0}")]
    UnknownOp(String),
    /// A storage backend failed (disk full, corrupt database, ...).
    #[error("storage error: {0}")]
    Storage(String),
}

impl Error {
    /// A stable, machine-readable code for non-Rust consumers.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Error::InvalidInput(_) => "invalid_input",
            Error::Parse(_) => "parse_error",
            Error::UnknownOp(_) => "unknown_op",
            Error::Storage(_) => "storage_error",
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::InvalidInput(e.to_string())
    }
}
