use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by msgkit. Individual malformed records are reported as
/// warnings in an [`ImportReport`](crate::ImportReport) instead.
///
/// `#[non_exhaustive]`: new kinds of error may be added in a minor release, so a `match` needs a catch-all arm
/// (or use [`Error::code`]).
#[derive(Debug, Error)]
#[non_exhaustive]
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
    /// The input is larger or deeper than a configured [`Limits`](crate::limits::Limits). Nothing was imported.
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
}

impl Error {
    /// A stable, machine-readable code for non-Rust consumers.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Error::InvalidInput(_) => codes::INVALID_INPUT,
            Error::Parse(_) => codes::PARSE_ERROR,
            Error::UnknownOp(_) => codes::UNKNOWN_OP,
            Error::Storage(_) => codes::STORAGE_ERROR,
            Error::LimitExceeded(_) => codes::LIMIT_EXCEEDED,
        }
    }
}

/// The stable error codes, shared by every binding and by the HTTP service (see `docs/ERRORS.md`).
///
/// The set only grows: a code is never renamed or reused for a different situation, and callers should treat a
/// code they do not know as a generic failure.
pub mod codes {
    /// The request could not be used: wrong JSON shape, missing field, bad option value, unusable query.
    pub const INVALID_INPUT: &str = "invalid_input";
    /// Well-formed input that is not a supported export or webhook.
    pub const PARSE_ERROR: &str = "parse_error";
    /// The operation name is not one of the supported operations.
    pub const UNKNOWN_OP: &str = "unknown_op";
    /// A store backend failed (disk full, corrupt database, ...).
    pub const STORAGE_ERROR: &str = "storage_error";
    /// An unexpected failure inside msgkit or at the native boundary (a caught panic, no usable result).
    pub const INTERNAL_ERROR: &str = "internal_error";
    /// The input is larger or deeper than a configured limit (the library's `Limits`, or the HTTP service's body cap).
    pub const LIMIT_EXCEEDED: &str = "limit_exceeded";
    /// HTTP: the bearer token is missing or wrong.
    pub const UNAUTHORIZED: &str = "unauthorized";
    /// HTTP: a webhook delivery whose signature does not match.
    pub const SIGNATURE_INVALID: &str = "signature_invalid";
    /// HTTP: the webhook subscription handshake failed.
    pub const VERIFICATION_FAILED: &str = "verification_failed";
    /// HTTP: the webhook is not configured on this service.
    pub const NOT_CONFIGURED: &str = "not_configured";

    /// Every code, in documentation order.
    pub const ALL: &[&str] = &[
        INVALID_INPUT,
        PARSE_ERROR,
        UNKNOWN_OP,
        STORAGE_ERROR,
        INTERNAL_ERROR,
        LIMIT_EXCEEDED,
        UNAUTHORIZED,
        SIGNATURE_INVALID,
        VERIFICATION_FAILED,
        NOT_CONFIGURED,
    ];
}

/// Describes a JSON error without repeating any of the input.
///
/// `serde_json` messages quote the offending value (`invalid type: string "..."`, `unknown variant
/// "..."`), and message content must never reach error text or logs. Only the error category, the name of
/// a missing field (a constant of the data contract, not user data) and, when `with_position` is set, the
/// line and column are kept.
#[must_use]
pub fn describe_json_error(e: &serde_json::Error, with_position: bool) -> String {
    use serde_json::error::Category;
    let kind = match e.classify() {
        Category::Io => "could not read the JSON input",
        Category::Syntax => "invalid JSON syntax",
        Category::Eof => "unexpected end of JSON input",
        Category::Data => "the JSON does not match the expected shape",
    };
    let text = e.to_string();
    let missing = text
        .strip_prefix("missing field `")
        .and_then(|rest| rest.split_once('`'))
        .filter(|(name, _)| name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .map(|(name, _)| format!(" (missing field `{name}`)"))
        .unwrap_or_default();
    if !with_position || e.line() == 0 {
        // Errors from an already-parsed value (`from_value`) have no position.
        format!("{kind}{missing}")
    } else {
        format!("{kind}{missing} at line {} column {}", e.line(), e.column())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        // The request document is assembled by a language binding, so a position inside it would differ
        // between bindings (key order, spacing) and tell the caller nothing about their own data.
        Error::InvalidInput(describe_json_error(&e, false))
    }
}
