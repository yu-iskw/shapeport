//! Structured diagnostics shared by CLI and MCP.

use serde::{Deserialize, Serialize};

use crate::path::FieldPath;

/// Diagnostic severity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// Machine-readable diagnostic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<FieldPath>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Diagnostic {
    #[must_use]
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code: code.into(),
            message: message.into(),
            path: None,
            hint: None,
        }
    }

    #[must_use]
    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code: code.into(),
            message: message.into(),
            path: None,
            hint: None,
        }
    }

    #[must_use]
    pub fn with_path(mut self, path: FieldPath) -> Self {
        self.path = Some(path);
        self
    }

    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{Diagnostic, Severity};

    #[test]
    fn serializes_camel_case() {
        let json = serde_json::to_string(&Diagnostic::warning("w", "hello")).expect("json");
        assert!(json.contains("\"severity\":\"warning\""));
        assert_eq!(Severity::Info, Severity::Info);
    }
}
