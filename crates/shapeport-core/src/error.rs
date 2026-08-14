//! Typed errors with stable CLI/MCP exit-code categories.

use std::fmt::{Display, Formatter};
use std::io;

/// Stable ShapePort error categories. Numeric values match CLI exit codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Usage = 2,
    Parse = 3,
    Schema = 4,
    Ambiguity = 5,
    PlanValidation = 6,
    Transform = 7,
    TargetValidation = 8,
    Security = 9,
    ResourceLimit = 10,
    Io = 11,
    Internal = 12,
}

impl ErrorKind {
    #[must_use]
    pub const fn exit_code(self) -> i32 {
        self as i32
    }
}

impl Display for ErrorKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let label = match *self {
            Self::Usage => "usage",
            Self::Parse => "parse",
            Self::Schema => "schema",
            Self::Ambiguity => "ambiguity",
            Self::PlanValidation => "plan_validation",
            Self::Transform => "transform",
            Self::TargetValidation => "target_validation",
            Self::Security => "security",
            Self::ResourceLimit => "resource_limit",
            Self::Io => "io",
            Self::Internal => "internal",
        };
        formatter.write_str(label)
    }
}

/// ShapePort error with kind, machine code, and human message.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{kind}: {code}: {message}")]
pub struct Error {
    pub kind: ErrorKind,
    pub code: String,
    pub message: String,
}

impl Error {
    #[must_use]
    pub fn new(kind: ErrorKind, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: code.into(),
            message: message.into(),
        }
    }

    #[must_use]
    pub fn usage(code: &str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Usage, code, message)
    }

    #[must_use]
    pub fn parse(code: &str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Parse, code, message)
    }

    #[must_use]
    pub fn schema(code: &str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Schema, code, message)
    }

    #[must_use]
    pub fn ambiguity(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Ambiguity, "ambiguous_mapping", message)
    }

    #[must_use]
    pub fn plan(code: &str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::PlanValidation, code, message)
    }

    #[must_use]
    pub fn transform(code: &str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Transform, code, message)
    }

    #[must_use]
    pub fn target(code: &str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::TargetValidation, code, message)
    }

    #[must_use]
    pub fn security(code: &str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Security, code, message)
    }

    #[must_use]
    pub fn limit(code: &str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::ResourceLimit, code, message)
    }

    #[must_use]
    pub fn io_err(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Io, "io_error", message)
    }

    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, "internal", message)
    }
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::io_err(value.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::parse("json_error", value.to_string())
    }
}

impl From<csv::Error> for Error {
    fn from(value: csv::Error) -> Self {
        Self::parse("csv_error", value.to_string())
    }
}

/// Result alias for ShapePort operations.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::{Error, ErrorKind};

    #[test]
    fn exit_codes_match_rfc() {
        assert_eq!(ErrorKind::Usage.exit_code(), 2);
        assert_eq!(ErrorKind::Ambiguity.exit_code(), 5);
        assert_eq!(ErrorKind::ResourceLimit.exit_code(), 10);
        assert_eq!(ErrorKind::Internal.exit_code(), 12);
    }

    #[test]
    fn display_includes_kind_and_code() {
        let err = Error::schema("type_mismatch", "expected number");
        assert!(err.to_string().contains("schema"));
        assert!(err.to_string().contains("type_mismatch"));
    }
}
