//! Filesystem roots, URI policy, resource checks, and artifact store.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::config::RuntimeConfig;
use crate::error::{Error, Result};

/// Metadata returned by `write_artifact`.
pub struct ArtifactMeta {
    /// SHA-256 digest as 64 lowercase hex characters (no prefix).
    pub digest: String,
    /// Filesystem path where the artifact was written.
    pub path: PathBuf,
    /// Size of the artifact in bytes.
    pub bytes: u64,
    /// Expiry time as an RFC 3339 string.
    pub expires_at: String,
}

/// Write bytes to the artifact store and return metadata.
///
/// The artifact is stored under `<first_write_root>/artifacts/<digest>`.
/// Returns `Error::limit` if the payload exceeds `config.mcp.artifact_max_bytes`.
pub fn write_artifact(bytes: &[u8], config: &RuntimeConfig) -> Result<ArtifactMeta> {
    let max = config.mcp.artifact_max_bytes;
    if bytes.len() as u64 > max {
        return Err(Error::limit(
            "artifact_max_bytes",
            format!("artifact size {} exceeds limit {max}", bytes.len()),
        ));
    }
    let digest = compute_digest(bytes);
    let dir = artifact_dir(config)?;
    fs::create_dir_all(&dir)?;
    let path = dir.join(&digest);
    fs::write(&path, bytes)?;

    let mtime = fs::metadata(&path)?.modified()?;
    let expires_at = mtime_plus_ttl(mtime, config.mcp.artifact_ttl_secs);
    Ok(ArtifactMeta {
        digest,
        path,
        bytes: bytes.len() as u64,
        expires_at,
    })
}

/// Read an artifact by URI (`shapeport-artifact://<64-hex-digest>`).
///
/// Returns `Error::security` for a malformed URI and `Error::limit` with
/// code `artifact_expired` when the artifact has expired (TTL elapsed).
pub fn read_artifact(uri: &str, config: &RuntimeConfig) -> Result<Vec<u8>> {
    let digest = parse_artifact_uri(uri)?;
    let path = artifact_dir(config)?.join(&digest);
    let meta = fs::metadata(&path).map_err(|_| {
        Error::security(
            "artifact_not_found",
            format!("artifact not found: {digest}"),
        )
    })?;
    let mtime = meta.modified()?;
    let ttl = config.mcp.artifact_ttl_secs;
    if is_expired(mtime, ttl) {
        return Err(Error::limit(
            "artifact_expired",
            format!("artifact {digest} has expired"),
        ));
    }
    Ok(fs::read(&path)?)
}

/// Resolve `file://` or plain paths, canonicalize, and enforce read roots.
pub fn resolve_read_path(raw: &str, config: &RuntimeConfig) -> Result<PathBuf> {
    let path = strip_file_uri(raw)?;
    enforce_root(&path, &config.filesystem.read_roots, "read")
}

/// Resolve a write path under configured write roots.
pub fn resolve_write_path(raw: &str, config: &RuntimeConfig) -> Result<PathBuf> {
    let path = strip_file_uri(raw)?;
    enforce_root(&path, &config.filesystem.write_roots, "write")
}

/// Read bytes with an input-size limit.
pub fn read_limited(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let meta = fs::metadata(path)?;
    if meta.len() > max_bytes {
        return Err(Error::limit(
            "max_input_bytes",
            format!("input {} exceeds maxInputBytes {max_bytes}", path.display()),
        ));
    }
    Ok(fs::read(path)?)
}

/// Strip `file://` prefix and reject unknown URI schemes.
///
/// The `shapeport-artifact://` scheme is NOT allowed through here — artifact
/// URIs must be handled explicitly by `read_artifact`.
fn strip_file_uri(raw: &str) -> Result<PathBuf> {
    if let Some(rest) = raw.strip_prefix("file://") {
        if rest.starts_with("http") {
            return Err(Error::security("uri_denied", "http(s) URIs are disabled"));
        }
        return Ok(PathBuf::from(rest));
    }
    if raw.contains("://") {
        return Err(Error::security(
            "uri_denied",
            format!("URI scheme is not allowed: {raw}"),
        ));
    }
    Ok(PathBuf::from(raw))
}

fn enforce_root(path: &Path, roots: &[PathBuf], access: &str) -> Result<PathBuf> {
    if roots.is_empty() {
        return canonicalize_or_parent(path);
    }
    let canonical = canonicalize_or_parent(path)?;
    let allowed = roots.iter().any(|root| {
        canonicalize_or_parent(root)
            .ok()
            .is_some_and(|root| canonical.starts_with(root))
    });
    if allowed {
        return Ok(canonical);
    }
    Err(Error::security(
        "path_denied",
        format!(
            "{access} path {} is outside configured roots",
            path.display()
        ),
    ))
}

fn canonicalize_or_parent(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    if let Some(parent) = path.parent() {
        if parent.as_os_str().is_empty() {
            let cwd = std::env::current_dir()?;
            return Ok(cwd.join(path));
        }
        if parent.exists() {
            let parent = parent.canonicalize()?;
            if let Some(name) = path.file_name() {
                return Ok(parent.join(name));
            }
        }
    }
    Ok(path.to_path_buf())
}

pub(crate) fn compute_digest(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let hash = sha2::Sha256::digest(bytes);
    hex::encode(hash)
}

fn artifact_dir(config: &RuntimeConfig) -> Result<PathBuf> {
    let root = config.filesystem.write_roots.first().ok_or_else(|| {
        Error::security("no_write_root", "no write root configured for artifacts")
    })?;
    Ok(root.join("artifacts"))
}

fn parse_artifact_uri(uri: &str) -> Result<String> {
    let digest = uri.strip_prefix("shapeport-artifact://").ok_or_else(|| {
        Error::security(
            "invalid_artifact_uri",
            "artifact URI must start with shapeport-artifact://",
        )
    })?;
    if digest.len() != 64
        || !digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
    {
        return Err(Error::security(
            "invalid_artifact_uri",
            "artifact URI digest must be 64 lowercase hex characters",
        ));
    }
    Ok(digest.to_string())
}

fn is_expired(mtime: std::time::SystemTime, ttl_secs: u64) -> bool {
    if ttl_secs == 0 {
        return true;
    }
    let elapsed = std::time::SystemTime::now()
        .duration_since(mtime)
        .unwrap_or(std::time::Duration::MAX);
    elapsed.as_secs() >= ttl_secs
}

fn mtime_plus_ttl(mtime: std::time::SystemTime, ttl_secs: u64) -> String {
    let dt: DateTime<Utc> = mtime.into();
    let expiry = dt + chrono::Duration::seconds(ttl_secs.cast_signed());
    expiry.to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::{compute_digest, read_artifact, strip_file_uri, write_artifact};
    use crate::config::RuntimeConfig;

    fn test_write_root() -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("shapeport_test_{}", std::process::id()));
        dir
    }

    #[test]
    fn rejects_http() {
        assert!(strip_file_uri("https://example.com/x").is_err());
        assert!(strip_file_uri("file:///tmp/x").is_ok());
    }

    #[test]
    fn rejects_artifact_scheme_in_strip_file_uri() {
        assert!(strip_file_uri("shapeport-artifact://abc").is_err());
    }

    #[test]
    fn artifact_roundtrip() {
        let root = test_write_root();
        let mut config = RuntimeConfig::default();
        config.filesystem.write_roots = vec![root];
        config.mcp.artifact_ttl_secs = 3600;

        let data = b"hello artifact";
        let meta = write_artifact(data, &config).expect("write");
        assert_eq!(meta.bytes, data.len() as u64);
        assert_eq!(meta.digest.len(), 64);
        assert_eq!(meta.digest, compute_digest(data));

        let uri = format!("shapeport-artifact://{}", meta.digest);
        let read_back = read_artifact(&uri, &config).expect("read");
        assert_eq!(read_back, data);
    }

    #[test]
    fn artifact_expired_with_zero_ttl() {
        let root = test_write_root();
        let mut config = RuntimeConfig::default();
        config.filesystem.write_roots = vec![root];
        config.mcp.artifact_ttl_secs = 0;

        let data = b"will expire";
        let meta = write_artifact(data, &config).expect("write");
        let uri = format!("shapeport-artifact://{}", meta.digest);
        let err = read_artifact(&uri, &config).expect_err("should expire");
        assert_eq!(err.code, "artifact_expired");
    }
}
