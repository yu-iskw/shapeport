//! MCP 2026-07-28 server built on `rmcp` 3.x.

use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{Implementation, ProtocolVersion, ServerCapabilities, ServerInfo};
use rmcp::transport::stdio;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use shapeport_core::config::{InferMode, RuntimeConfig};
use shapeport_core::formats::FormatId;
use shapeport_core::plan::{ErrorPolicy, TransformationPlan, parse_plan_json};
use shapeport_core::{
    ConvertRequest, InspectRequest, PlanRequest, PlannerMode, QueryRequest, SourceSpec,
    TransformRequest, ValidateRequest, Value, convert_data, inspect_source, plan_mapping,
    query_sources, schema_as_json, schema_fingerprint, schema_from_json_value, transform_data,
    validate_data, write_artifact,
};
use tokio::net::TcpListener;

#[derive(Clone)]
pub struct AppState {
    pub config: RuntimeConfig,
}

#[derive(Clone)]
pub struct ShapePortMcp {
    pub state: AppState,
}

impl ShapePortMcp {
    #[must_use]
    pub const fn new(config: RuntimeConfig) -> Self {
        Self {
            state: AppState { config },
        }
    }
}

/// Reference to a data source: either a URI or inline JSON.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    /// `file://` URI or `shapeport-artifact://` URI. One of `uri` or `inline` is required.
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub inline: Option<JsonValue>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InspectArgs {
    pub source: SourceRef,
    #[serde(default)]
    pub options: InspectOptions,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InspectOptions {
    #[serde(default)]
    pub sample_rows: Option<u32>,
    #[serde(default)]
    pub infer_types: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InspectOut {
    pub format: FormatOut,
    pub schema: JsonValue,
    pub schema_fingerprint: String,
    pub statistics: StatsOut,
    pub sample: JsonValue,
    pub diagnostics: Vec<JsonValue>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FormatOut {
    pub name: String,
    pub confidence: f64,
    pub evidence: Vec<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatsOut {
    pub rows: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SchemaArgs {
    #[serde(default)]
    pub source: Option<SourceRef>,
    #[serde(default)]
    pub schema: Option<JsonValue>,
    #[serde(default)]
    pub as_dialect: Option<String>,
    #[serde(rename = "as", default)]
    pub as_field: Option<String>,
    #[serde(default)]
    pub infer_types: Option<String>,
    #[serde(default)]
    pub sample_rows: Option<u32>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SchemaOut {
    pub schema: JsonValue,
    pub fingerprint: String,
    pub dialect: String,
    pub diagnostics: Vec<JsonValue>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlanArgs {
    #[serde(default)]
    pub source_schema: Option<JsonValue>,
    #[serde(default)]
    pub source: Option<SourceRef>,
    pub target_schema: JsonValue,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub explain: Option<bool>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlanOut {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<JsonValue>,
    pub explanation: JsonValue,
    pub unresolved: JsonValue,
    pub diagnostics: Vec<JsonValue>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TransformArgs {
    pub source: SourceRef,
    #[serde(default)]
    pub plan: Option<JsonValue>,
    #[serde(default)]
    pub target_schema: Option<JsonValue>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub output_format: Option<String>,
    #[serde(default)]
    pub error_policy: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DataOut {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ArtifactOut>,
    /// Summary of the operation: rows, bytes, format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<JsonValue>,
    pub diagnostics: Vec<JsonValue>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactOut {
    pub uri: String,
    pub format: String,
    pub bytes: u64,
    pub rows: u64,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Filesystem path; only set when `config.mcp.local_filesystem` is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolErrorOut {
    pub status: String,
    pub kind: String,
    pub code: String,
    pub message: String,
    pub diagnostics: Vec<JsonValue>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidateArgs {
    #[serde(default)]
    pub source: Option<SourceRef>,
    #[serde(default)]
    pub schema: Option<JsonValue>,
    #[serde(default)]
    pub plan: Option<JsonValue>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidateOut {
    pub valid: bool,
    pub errors: Vec<JsonValue>,
    pub diagnostics: Vec<JsonValue>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueryArgs {
    pub sql: String,
    #[serde(default)]
    pub sources: Option<std::collections::HashMap<String, SourceRef>>,
    #[serde(default)]
    pub source: Option<SourceRef>,
    #[serde(default)]
    pub output_format: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConvertArgs {
    pub source: SourceRef,
    pub to: String,
}

#[tool_router]
impl ShapePortMcp {
    #[tool(
        name = "shapeport_inspect",
        description = "Inspect format, schema, and statistics of a data source"
    )]
    fn inspect(
        &self,
        Parameters(args): Parameters<InspectArgs>,
    ) -> Result<Json<InspectOut>, Json<ToolErrorOut>> {
        let infer = parse_infer(args.options.infer_types.as_deref());
        let result = inspect_source(
            &InspectRequest {
                source: to_source(&args.source).map_err(tool_fail)?,
                infer,
                sample_rows: args.options.sample_rows.unwrap_or(20) as usize,
            },
            &self.state.config,
        )
        .map_err(tool_fail)?;
        Ok(Json(InspectOut {
            format: FormatOut {
                name: result.detection.format.as_str().into(),
                confidence: result.detection.confidence,
                evidence: result.detection.evidence,
            },
            schema: serde_json::to_value(&result.schema).unwrap_or(JsonValue::Null),
            schema_fingerprint: result.fingerprint,
            statistics: StatsOut {
                rows: result.row_count as u64,
            },
            sample: serde_json::to_value(&result.sample).unwrap_or(JsonValue::Null),
            diagnostics: Vec::new(),
        }))
    }

    #[tool(name = "shapeport_schema", description = "Infer or convert a schema")]
    fn schema_tool(
        &self,
        Parameters(args): Parameters<SchemaArgs>,
    ) -> Result<Json<SchemaOut>, Json<ToolErrorOut>> {
        let dialect = args
            .as_field
            .or(args.as_dialect)
            .unwrap_or_else(|| "shapeport".into());
        if let Some(schema) = args.schema {
            let parsed = schema_from_json_value(&schema).map_err(tool_fail)?;
            return Ok(Json(emit_schema(&parsed, &dialect)));
        }
        let source = args.source.ok_or_else(|| {
            tool_fail(shapeport_core::Error::usage(
                "missing_source",
                "source or schema is required",
            ))
        })?;
        let inspected = inspect_source(
            &InspectRequest {
                source: to_source(&source).map_err(tool_fail)?,
                infer: parse_infer(args.infer_types.as_deref()),
                sample_rows: args.sample_rows.unwrap_or(100) as usize,
            },
            &self.state.config,
        )
        .map_err(tool_fail)?;
        Ok(Json(emit_schema(&inspected.schema, &dialect)))
    }

    #[tool(
        name = "shapeport_plan",
        description = "Build a deterministic Transformation Plan"
    )]
    fn plan(
        &self,
        Parameters(args): Parameters<PlanArgs>,
    ) -> Result<Json<PlanOut>, Json<ToolErrorOut>> {
        let target = schema_from_json_value(&args.target_schema).map_err(tool_fail)?;
        let source_schema = args
            .source_schema
            .as_ref()
            .map(schema_from_json_value)
            .transpose()
            .map_err(tool_fail)?;
        let planned = plan_mapping(
            &PlanRequest {
                source_schema,
                target_schema: target,
                source: args
                    .source
                    .as_ref()
                    .map(to_source)
                    .transpose()
                    .map_err(tool_fail)?,
                mode: parse_mode(args.mode.as_deref()),
                infer: InferMode::Conservative,
            },
            &self.state.config,
        )
        .map_err(tool_fail)?;
        Ok(Json(PlanOut {
            status: planned.status,
            plan: planned
                .plan
                .map(|plan| serde_json::to_value(plan).unwrap_or(JsonValue::Null)),
            explanation: serde_json::to_value(&planned.explanation).unwrap_or(JsonValue::Null),
            unresolved: serde_json::to_value(&planned.unresolved).unwrap_or(JsonValue::Null),
            diagnostics: Vec::new(),
        }))
    }

    #[tool(
        name = "shapeport_transform",
        description = "Execute a Transformation Plan or plan from a target schema"
    )]
    fn transform(
        &self,
        Parameters(args): Parameters<TransformArgs>,
    ) -> Result<Json<DataOut>, Json<ToolErrorOut>> {
        let fmt = parse_format(args.output_format.as_deref(), FormatId::Json);
        let result = transform_data(
            &TransformRequest {
                source: to_source(&args.source).map_err(tool_fail)?,
                plan: args
                    .plan
                    .as_ref()
                    .map(parse_plan_value)
                    .transpose()
                    .map_err(tool_fail)?,
                target_schema: args
                    .target_schema
                    .as_ref()
                    .map(schema_from_json_value)
                    .transpose()
                    .map_err(tool_fail)?,
                mode: parse_mode(args.mode.as_deref()),
                output_format: fmt,
                error_policy: parse_policy(args.error_policy.as_deref()),
                infer: InferMode::Conservative,
            },
            &self.state.config,
        )
        .map_err(tool_fail)?;
        pack_data(&result, fmt, &self.state.config)
    }

    #[tool(
        name = "shapeport_validate",
        description = "Validate data against a schema, or validate a plan"
    )]
    fn validate(
        &self,
        Parameters(args): Parameters<ValidateArgs>,
    ) -> Result<Json<ValidateOut>, Json<ToolErrorOut>> {
        let result = validate_data(
            &ValidateRequest {
                source: args
                    .source
                    .as_ref()
                    .map(to_source)
                    .transpose()
                    .map_err(tool_fail)?,
                schema: args
                    .schema
                    .as_ref()
                    .map(schema_from_json_value)
                    .transpose()
                    .map_err(tool_fail)?,
                plan: args
                    .plan
                    .as_ref()
                    .map(parse_plan_value)
                    .transpose()
                    .map_err(tool_fail)?,
                infer: InferMode::Conservative,
            },
            &self.state.config,
        )
        .map_err(tool_fail)?;
        Ok(Json(ValidateOut {
            valid: result.valid,
            errors: result
                .errors
                .into_iter()
                .map(|d| serde_json::to_value(d).unwrap_or(JsonValue::Null))
                .collect(),
            diagnostics: Vec::new(),
        }))
    }

    #[tool(
        name = "shapeport_query",
        description = "Run bounded SQL over explicitly registered sources"
    )]
    fn query(
        &self,
        Parameters(args): Parameters<QueryArgs>,
    ) -> Result<Json<DataOut>, Json<ToolErrorOut>> {
        let mut sources = std::collections::HashMap::new();
        if let Some(map) = args.sources {
            for (name, source) in map {
                sources.insert(name, to_source(&source).map_err(tool_fail)?);
            }
        }
        if let Some(source) = args.source {
            sources.insert("input".into(), to_source(&source).map_err(tool_fail)?);
        }
        let format = parse_format(args.output_format.as_deref(), FormatId::Json);
        let result = query_sources(
            &QueryRequest {
                sql: args.sql,
                sources,
                output_format: format,
                infer: InferMode::Conservative,
            },
            &self.state.config,
        )
        .map_err(tool_fail)?;
        pack_data(&result, format, &self.state.config)
    }

    #[tool(
        name = "shapeport_convert",
        description = "Convert representation with minimal reshaping"
    )]
    fn convert(
        &self,
        Parameters(args): Parameters<ConvertArgs>,
    ) -> Result<Json<DataOut>, Json<ToolErrorOut>> {
        let to = FormatId::parse(&args.to).ok_or_else(|| {
            tool_fail(shapeport_core::Error::usage(
                "unknown_format",
                "unknown output format",
            ))
        })?;
        let result = convert_data(
            &ConvertRequest {
                source: to_source(&args.source).map_err(tool_fail)?,
                to,
                infer: InferMode::Conservative,
            },
            &self.state.config,
        )
        .map_err(tool_fail)?;
        pack_data(&result, to, &self.state.config)
    }
}

#[tool_handler(
    name = "shapeport",
    version = "0.1.0",
    instructions = "Schema-driven data transformation runtime. Prefer inspect → plan → transform. Large results return shapeport-artifact:// URIs."
)]
impl ServerHandler for ShapePortMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2026_07_28)
            .with_server_info(Implementation::new("shapeport", "0.1.0"))
            .with_instructions(
                "Schema-driven data transformation runtime. \
                 Prefer inspect → plan → transform. \
                 Large results return shapeport-artifact:// URIs.",
            )
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25])
    }
}

fn emit_schema(schema: &shapeport_core::Schema, dialect: &str) -> SchemaOut {
    let fingerprint = schema_fingerprint(schema);
    let rendered = if dialect == "json-schema" {
        schema_as_json(schema)
    } else {
        serde_json::to_value(schema).unwrap_or(JsonValue::Null)
    };
    SchemaOut {
        schema: rendered,
        fingerprint,
        dialect: dialect.into(),
        diagnostics: Vec::new(),
    }
}

fn collect_diagnostics(diags: &[shapeport_core::Diagnostic]) -> Vec<JsonValue> {
    diags
        .iter()
        .map(|d| serde_json::to_value(d).unwrap_or(JsonValue::Null))
        .collect()
}

fn make_receipt(rows: u64, bytes: u64, format: FormatId) -> JsonValue {
    serde_json::json!({
        "rows": rows,
        "bytes": bytes,
        "format": format.as_str(),
    })
}

const fn is_binary_format(format: FormatId) -> bool {
    matches!(format, FormatId::Parquet | FormatId::ArrowIpc)
}

fn pack_data(
    result: &shapeport_core::TransformResult,
    format: FormatId,
    config: &RuntimeConfig,
) -> Result<Json<DataOut>, Json<ToolErrorOut>> {
    let rows = result.records.len() as u64;
    let bytes = result.bytes.len() as u64;
    let diagnostics = collect_diagnostics(&result.diagnostics);
    if is_binary_format(format)
        || bytes > config.mcp.inline_max_bytes
        || rows > config.mcp.inline_max_rows
    {
        return pack_artifact(result, format, config, rows, bytes, diagnostics);
    }
    let payload = if format == FormatId::Json || format == FormatId::Jsonl {
        serde_json::to_value(&result.records).unwrap_or(JsonValue::Null)
    } else {
        JsonValue::String(String::from_utf8_lossy(&result.bytes).into_owned())
    };
    Ok(Json(DataOut {
        status: "ok".into(),
        result: Some(payload),
        artifact: None,
        receipt: Some(make_receipt(rows, bytes, format)),
        diagnostics,
    }))
}

fn pack_artifact(
    result: &shapeport_core::TransformResult,
    format: FormatId,
    config: &RuntimeConfig,
    rows: u64,
    bytes: u64,
    diagnostics: Vec<JsonValue>,
) -> Result<Json<DataOut>, Json<ToolErrorOut>> {
    let meta = write_artifact(&result.bytes, config).map_err(tool_fail)?;
    let schema_fp = result.schema.as_ref().map(schema_fingerprint);
    let local_path = if config.mcp.local_filesystem {
        Some(meta.path.to_string_lossy().into_owned())
    } else {
        None
    };
    Ok(Json(DataOut {
        status: "ok".into(),
        result: None,
        artifact: Some(ArtifactOut {
            uri: format!("shapeport-artifact://{}", meta.digest),
            format: format.as_str().into(),
            bytes,
            rows,
            sha256: meta.digest,
            schema_fingerprint: schema_fp,
            expires_at: Some(meta.expires_at),
            local_path,
        }),
        receipt: Some(make_receipt(rows, bytes, format)),
        diagnostics,
    }))
}

fn to_source(src: &SourceRef) -> Result<SourceSpec, shapeport_core::Error> {
    Ok(SourceSpec {
        uri: src.uri.clone(),
        format: src.format.as_deref().and_then(FormatId::parse),
        inline: src
            .inline
            .as_ref()
            .map(|value| serde_json::from_value::<Value>(value.clone()))
            .transpose()
            .map_err(|e| shapeport_core::Error::parse("invalid_inline", e.to_string()))?,
        bytes: None,
    })
}

fn parse_plan_value(value: &JsonValue) -> Result<TransformationPlan, shapeport_core::Error> {
    let raw = serde_json::to_string(value)?;
    parse_plan_json(&raw)
}

fn parse_infer(raw: Option<&str>) -> InferMode {
    raw.and_then(InferMode::parse)
        .unwrap_or(InferMode::Conservative)
}

fn parse_mode(raw: Option<&str>) -> PlannerMode {
    raw.and_then(PlannerMode::parse)
        .unwrap_or(PlannerMode::Smart)
}

fn parse_format(raw: Option<&str>, default: FormatId) -> FormatId {
    raw.and_then(FormatId::parse).unwrap_or(default)
}

fn parse_policy(raw: Option<&str>) -> ErrorPolicy {
    match raw {
        Some("skip") => ErrorPolicy::Skip,
        Some("collect") => ErrorPolicy::Collect,
        _ => ErrorPolicy::Fail,
    }
}

fn tool_fail(err: shapeport_core::Error) -> Json<ToolErrorOut> {
    Json(ToolErrorOut {
        status: "error".into(),
        kind: err.kind.to_string(),
        code: err.code,
        message: err.message,
        diagnostics: Vec::new(),
    })
}

/// Serve MCP over stdio. stdout is protocol-only.
pub async fn serve_stdio(config: RuntimeConfig) -> Result<(), String> {
    let server = ShapePortMcp::new(config);
    let running = server.serve(stdio()).await.map_err(|err| err.to_string())?;
    running.waiting().await.map_err(|err| err.to_string())?;
    Ok(())
}

/// Serve MCP over stateless Streamable HTTP.
///
/// Non-loopback binds require `config.mcp.bearer_token` to be set; callers
/// receive `401 Unauthorized` when no `Authorization: Bearer <token>` header
/// is present. When `bind` is unspecified (`0.0.0.0` / `::`), `Host`
/// validation is disabled because the public hostname is unknown at startup —
/// Bearer is still enforced.
pub async fn serve_http(bind: SocketAddr, config: RuntimeConfig) -> Result<(), String> {
    let require_auth = !bind.ip().is_loopback();
    let token = config.mcp.bearer_token.clone();
    if require_auth && token.is_none() {
        return Err("non-loopback bind requires SHAPEPORT_MCP_TOKEN".into());
    }
    let http_config = build_http_config(bind, &config);
    let shared = Arc::new(config.clone());
    let service = StreamableHttpService::new(
        {
            let shared = Arc::clone(&shared);
            move || Ok(ShapePortMcp::new((*shared).clone()))
        },
        LocalSessionManager::default().into(),
        http_config,
    );
    let base = Router::new().nest_service("/mcp", service);
    let router = if require_auth {
        let auth_state = AuthState {
            token: token.unwrap_or_default(),
        };
        base.layer(middleware::from_fn_with_state(auth_state, auth_bearer))
    } else {
        base
    };
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|err| err.to_string())?;
    axum::serve(listener, router)
        .await
        .map_err(|err| err.to_string())
}

fn build_http_config(bind: SocketAddr, config: &RuntimeConfig) -> StreamableHttpServerConfig {
    let cfg = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_max_request_body_bytes(32 * 1024 * 1024);
    let cfg = if config.mcp.origin_allowlist.is_empty() {
        cfg
    } else {
        cfg.with_allowed_origins(config.mcp.origin_allowlist.clone())
    };
    configure_allowed_hosts(cfg, bind)
}

/// Configure `Host` validation for the given bind address.
///
/// - Loopback: keep rmcp defaults (`localhost`, `127.0.0.1`, `::1`).
/// - Unspecified (`0.0.0.0` / `::`): disable validation; the public hostname is
///   not known at startup and Bearer auth is still enforced.
/// - Specific non-loopback IP: allow the bind IP and `ip:port` in addition to
///   loopback names.
fn configure_allowed_hosts(
    cfg: StreamableHttpServerConfig,
    bind: SocketAddr,
) -> StreamableHttpServerConfig {
    let ip = bind.ip();
    if ip.is_loopback() {
        cfg
    } else if ip.is_unspecified() {
        cfg.disable_allowed_hosts()
    } else {
        cfg.with_allowed_hosts([
            "localhost".to_string(),
            "127.0.0.1".to_string(),
            "::1".to_string(),
            ip.to_string(),
            bind.to_string(),
        ])
    }
}

#[derive(Clone)]
struct AuthState {
    token: String,
}

async fn auth_bearer(
    axum::extract::State(state): axum::extract::State<AuthState>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response, StatusCode> {
    check_bearer(&headers, &state.token)?;
    Ok(next.run(request).await)
}

fn check_bearer(headers: &HeaderMap, expected: &str) -> Result<(), StatusCode> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let provided = header
        .strip_prefix("Bearer ")
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if provided != expected {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ConvertRequest, FormatId, InferMode, JsonValue, RuntimeConfig, SocketAddr, SourceSpec,
        Value, convert_data, pack_data, serve_http,
    };
    use shapeport_core::read_artifact;

    /// `serve_http` must return an error before binding when the address is
    /// non-loopback and no bearer token is configured.
    #[tokio::test]
    async fn serve_http_requires_token_for_non_loopback() {
        let bind: SocketAddr = "0.0.0.0:0".parse().expect("valid addr");
        let result = serve_http(bind, RuntimeConfig::default()).await;
        assert!(result.is_err(), "expected Err, got Ok");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("SHAPEPORT_MCP_TOKEN"),
            "expected token hint in message, got: {msg}"
        );
    }

    fn artifact_config() -> RuntimeConfig {
        let root = {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "shapeport_mcp_pack_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("time")
                    .as_nanos()
            ));
            path
        };
        let mut config = RuntimeConfig::default();
        config.filesystem.write_roots = vec![root.clone()];
        config.filesystem.read_roots = vec![root];
        config
    }

    fn tiny_json_source() -> SourceSpec {
        SourceSpec {
            uri: None,
            inline: Some(Value::Array(vec![Value::object([(
                "n".into(),
                Value::Int(1),
            )])])),
            format: Some(FormatId::Json),
            bytes: None,
        }
    }

    fn packed_ok(
        result: Result<super::Json<super::DataOut>, super::Json<super::ToolErrorOut>>,
        what: &str,
    ) -> super::Json<super::DataOut> {
        result.unwrap_or_else(|_| panic!("{what}"))
    }

    #[test]
    fn pack_data_artifacts_small_parquet() {
        let config = artifact_config();
        let converted = convert_data(
            &ConvertRequest {
                source: tiny_json_source(),
                to: FormatId::Parquet,
                infer: InferMode::Conservative,
            },
            &config,
        )
        .expect("convert");
        assert!(
            (converted.bytes.len() as u64) < config.mcp.inline_max_bytes,
            "fixture must stay under inline byte threshold"
        );
        let packed = packed_ok(
            pack_data(&converted, FormatId::Parquet, &config),
            "pack parquet",
        );
        assert!(packed.0.result.is_none(), "binary must not inline");
        let artifact = packed.0.artifact.expect("artifact");
        let stored = read_artifact(&artifact.uri, &config).expect("read");
        assert_eq!(stored, converted.bytes);
    }

    #[test]
    fn pack_data_artifacts_small_arrow_ipc() {
        let config = artifact_config();
        let converted = convert_data(
            &ConvertRequest {
                source: tiny_json_source(),
                to: FormatId::ArrowIpc,
                infer: InferMode::Conservative,
            },
            &config,
        )
        .expect("convert");
        let packed = packed_ok(
            pack_data(&converted, FormatId::ArrowIpc, &config),
            "pack arrow-ipc",
        );
        assert!(packed.0.result.is_none());
        let artifact = packed.0.artifact.expect("artifact");
        let stored = read_artifact(&artifact.uri, &config).expect("read");
        assert_eq!(stored, converted.bytes);
    }

    #[test]
    fn pack_data_keeps_small_csv_inline() {
        let config = artifact_config();
        let converted = convert_data(
            &ConvertRequest {
                source: tiny_json_source(),
                to: FormatId::Csv,
                infer: InferMode::Conservative,
            },
            &config,
        )
        .expect("convert");
        let packed = packed_ok(pack_data(&converted, FormatId::Csv, &config), "pack csv");
        assert!(packed.0.artifact.is_none());
        let Some(JsonValue::String(text)) = packed.0.result else {
            panic!("expected inline CSV string");
        };
        assert!(text.contains('n'), "csv header: {text}");
    }
}
