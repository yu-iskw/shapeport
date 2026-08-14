//! Runtime configuration (RFC 0001 §22).

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

/// Inference mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InferMode {
    None,
    #[default]
    Conservative,
    Aggressive,
}

impl InferMode {
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "none" => Some(Self::None),
            "conservative" => Some(Self::Conservative),
            "aggressive" => Some(Self::Aggressive),
            _ => None,
        }
    }
}

/// Resource limits.
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_input_bytes: u64,
    pub max_output_bytes: u64,
    pub max_schema_depth: u32,
    pub max_nesting_depth: u32,
    pub max_rows: u64,
    pub max_inline_bytes: u64,
    pub max_inline_rows: u64,
    pub timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 10 * 1024 * 1024 * 1024,
            max_output_bytes: 10 * 1024 * 1024 * 1024,
            max_schema_depth: 64,
            max_nesting_depth: 128,
            max_rows: 50_000_000,
            max_inline_bytes: 1_048_576,
            max_inline_rows: 1_000,
            timeout: Duration::from_secs(300),
        }
    }
}

/// Filesystem roots.
#[derive(Clone, Debug, Default)]
pub struct FilesystemPolicy {
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
}

/// Planner thresholds.
#[derive(Clone, Debug)]
pub struct PlannerConfig {
    pub auto_accept_threshold: f64,
    pub ambiguity_threshold: f64,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            auto_accept_threshold: 0.95,
            ambiguity_threshold: 0.80,
        }
    }
}

/// MCP inline/artifact policy.
#[derive(Clone, Debug)]
pub struct McpConfig {
    pub inline_max_bytes: u64,
    pub inline_max_rows: u64,
    pub artifact_ttl_secs: u64,
    pub artifact_max_bytes: u64,
    pub local_filesystem: bool,
    pub origin_allowlist: Vec<String>,
    pub bearer_token: Option<String>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            inline_max_bytes: 1_048_576,
            inline_max_rows: 1_000,
            artifact_ttl_secs: 3600,
            artifact_max_bytes: 1_073_741_824,
            local_filesystem: false,
            origin_allowlist: Vec::new(),
            bearer_token: None,
        }
    }
}

/// Top-level runtime configuration.
#[derive(Clone, Debug, Default)]
pub struct RuntimeConfig {
    pub limits: Limits,
    pub filesystem: FilesystemPolicy,
    pub planner: PlannerConfig,
    pub inference: InferMode,
    pub sample_rows: usize,
    pub mcp: McpConfig,
    pub batch_rows: usize,
    pub null_spellings: Vec<String>,
}

impl RuntimeConfig {
    #[must_use]
    #[allow(clippy::needless_pass_by_value)]
    pub fn with_cwd_roots(mut self, cwd: PathBuf) -> Self {
        if self.filesystem.read_roots.is_empty() {
            self.filesystem.read_roots.push(cwd.clone());
        }
        if self.filesystem.write_roots.is_empty() {
            self.filesystem.write_roots.push(cwd.join(".shapeport"));
        }
        self
    }

    /// Load configuration from a YAML file, overlaying onto defaults.
    pub fn load_yaml(path: &std::path::Path) -> crate::error::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| crate::error::Error::io_err(e.to_string()))?;
        Self::from_yaml_str(&raw)
    }

    /// Parse configuration from a YAML string, overlaying onto defaults.
    pub fn from_yaml_str(raw: &str) -> crate::error::Result<Self> {
        let file: FileConfig = serde_norway::from_str(raw)
            .map_err(|e| crate::error::Error::parse("yaml_config", e.to_string()))?;
        Ok(file.into_runtime())
    }
}

// ── YAML file config structs ──────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FileConfig {
    inference: FileInference,
    security: FileSecurity,
    mcp: FileMcp,
}

impl FileConfig {
    fn into_runtime(self) -> RuntimeConfig {
        let mut cfg = RuntimeConfig::default();
        apply_inference(&self.inference, &mut cfg);
        apply_security(&self.security, &mut cfg);
        apply_mcp(&self.mcp, &mut cfg);
        cfg
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FileInference {
    mode: Option<String>,
    sample_rows: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FileSecurity {
    filesystem: FileFilesystem,
    limits: FileLimits,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FileFilesystem {
    read_roots: Option<Vec<String>>,
    write_roots: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FileLimits {
    max_input_bytes: Option<u64>,
    max_output_bytes: Option<u64>,
    max_schema_depth: Option<u32>,
    max_nesting_depth: Option<u32>,
    timeout_seconds: Option<u64>,
    max_rows: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FileMcp {
    allowed_origins: Option<Vec<String>>,
    inline_result: FileMcpInline,
    artifacts: FileMcpArtifacts,
    token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FileMcpInline {
    max_bytes: Option<u64>,
    max_rows: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FileMcpArtifacts {
    max_bytes: Option<u64>,
    ttl_seconds: Option<u64>,
}

fn apply_inference(file: &FileInference, cfg: &mut RuntimeConfig) {
    if let Some(mode_str) = &file.mode
        && let Some(mode) = InferMode::parse(mode_str)
    {
        cfg.inference = mode;
    }
    if let Some(rows) = file.sample_rows {
        cfg.sample_rows = rows;
    }
}

fn apply_security(file: &FileSecurity, cfg: &mut RuntimeConfig) {
    apply_filesystem(&file.filesystem, cfg);
    apply_limits(&file.limits, cfg);
}

fn apply_filesystem(file: &FileFilesystem, cfg: &mut RuntimeConfig) {
    if let Some(roots) = &file.read_roots {
        cfg.filesystem.read_roots = roots.iter().map(PathBuf::from).collect();
    }
    if let Some(roots) = &file.write_roots {
        cfg.filesystem.write_roots = roots.iter().map(PathBuf::from).collect();
    }
}

const fn apply_limits(file: &FileLimits, cfg: &mut RuntimeConfig) {
    if let Some(v) = file.max_input_bytes {
        cfg.limits.max_input_bytes = v;
    }
    if let Some(v) = file.max_output_bytes {
        cfg.limits.max_output_bytes = v;
    }
    if let Some(v) = file.max_schema_depth {
        cfg.limits.max_schema_depth = v;
    }
    if let Some(v) = file.max_nesting_depth {
        cfg.limits.max_nesting_depth = v;
    }
    if let Some(v) = file.timeout_seconds {
        cfg.limits.timeout = Duration::from_secs(v);
    }
    if let Some(v) = file.max_rows {
        cfg.limits.max_rows = v;
    }
}

fn apply_mcp(file: &FileMcp, cfg: &mut RuntimeConfig) {
    if let Some(origins) = &file.allowed_origins {
        cfg.mcp.origin_allowlist.clone_from(origins);
    }
    if let Some(v) = file.inline_result.max_bytes {
        cfg.mcp.inline_max_bytes = v;
    }
    if let Some(v) = file.inline_result.max_rows {
        cfg.mcp.inline_max_rows = v;
    }
    if let Some(v) = file.artifacts.max_bytes {
        cfg.mcp.artifact_max_bytes = v;
    }
    if let Some(v) = file.artifacts.ttl_seconds {
        cfg.mcp.artifact_ttl_secs = v;
    }
    if let Some(token) = &file.token {
        cfg.mcp.bearer_token = Some(token.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeConfig;

    #[test]
    fn from_yaml_str_inference_mode() {
        let yaml = "inference:\n  mode: aggressive\n  sampleRows: 200\n";
        let cfg = RuntimeConfig::from_yaml_str(yaml).expect("parse");
        assert_eq!(cfg.inference, super::InferMode::Aggressive);
        assert_eq!(cfg.sample_rows, 200);
    }

    #[test]
    fn from_yaml_str_limits() {
        let yaml = "security:\n  limits:\n    maxInputBytes: 1024\n    timeoutSeconds: 60\n    maxRows: 5000\n";
        let cfg = RuntimeConfig::from_yaml_str(yaml).expect("parse");
        assert_eq!(cfg.limits.max_input_bytes, 1024);
        assert_eq!(cfg.limits.timeout.as_secs(), 60);
        assert_eq!(cfg.limits.max_rows, 5000);
    }

    #[test]
    fn from_yaml_str_mcp_token() {
        let yaml = "mcp:\n  token: secret123\n";
        let cfg = RuntimeConfig::from_yaml_str(yaml).expect("parse");
        assert_eq!(cfg.mcp.bearer_token.as_deref(), Some("secret123"));
    }

    #[test]
    fn defaults_null_spellings_empty() {
        let cfg = RuntimeConfig::default();
        assert!(cfg.null_spellings.is_empty());
    }

    #[test]
    fn defaults_artifact_max_bytes() {
        let cfg = RuntimeConfig::default();
        assert_eq!(cfg.mcp.artifact_max_bytes, 1_073_741_824);
    }
}
