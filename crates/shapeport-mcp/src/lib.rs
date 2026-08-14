//! MCP 2026-07-28 server built on `rmcp` 3.x.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::transport::stdio;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use shapeport_core::config::{InferMode, RuntimeConfig};
use shapeport_core::formats::FormatId;
use shapeport_core::plan::{ErrorPolicy, TransformationPlan, parse_plan_json};
use shapeport_core::{
    ConvertRequest, InspectRequest, PlanRequest, PlannerMode, QueryRequest, SourceSpec,
    TransformRequest, ValidateRequest, Value, convert_data, inspect_source, plan_mapping,
    query_sources, schema_as_json, schema_from_json_value, transform_data, validate_data,
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
    pub fn new(config: RuntimeConfig) -> Self {
        Self {
            state: AppState { config },
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
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
    ) -> Result<Json<InspectOut>, McpError> {
        let infer = parse_infer(args.options.infer_types.as_deref());
        let result = inspect_source(
            &InspectRequest {
                source: to_source(&args.source)?,
                infer,
                sample_rows: args.options.sample_rows.unwrap_or(20) as usize,
            },
            &self.state.config,
        )
        .map_err(tool_err)?;
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
    ) -> Result<Json<SchemaOut>, McpError> {
        let dialect = args
            .as_field
            .or(args.as_dialect)
            .unwrap_or_else(|| "shapeport".into());
        if let Some(schema) = args.schema {
            let parsed = schema_from_json_value(&schema).map_err(tool_err)?;
            return Ok(Json(emit_schema(parsed, &dialect)));
        }
        let source = args
            .source
            .ok_or_else(|| McpError::invalid_params("source or schema is required", None))?;
        let inspected = inspect_source(
            &InspectRequest {
                source: to_source(&source)?,
                infer: parse_infer(args.infer_types.as_deref()),
                sample_rows: args.sample_rows.unwrap_or(100) as usize,
            },
            &self.state.config,
        )
        .map_err(tool_err)?;
        Ok(Json(emit_schema(inspected.schema, &dialect)))
    }

    #[tool(
        name = "shapeport_plan",
        description = "Build a deterministic Transformation Plan"
    )]
    fn plan(&self, Parameters(args): Parameters<PlanArgs>) -> Result<Json<PlanOut>, McpError> {
        let target = schema_from_json_value(&args.target_schema).map_err(tool_err)?;
        let source_schema = args
            .source_schema
            .as_ref()
            .map(schema_from_json_value)
            .transpose()
            .map_err(tool_err)?;
        let planned = plan_mapping(
            &PlanRequest {
                source_schema,
                target_schema: target,
                source: args.source.as_ref().map(to_source).transpose()?,
                mode: parse_mode(args.mode.as_deref()),
                infer: InferMode::Conservative,
            },
            &self.state.config,
        )
        .map_err(tool_err)?;
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
    ) -> Result<Json<DataOut>, McpError> {
        let result = transform_data(
            &TransformRequest {
                source: to_source(&args.source)?,
                plan: args.plan.as_ref().map(parse_plan_value).transpose()?,
                target_schema: args
                    .target_schema
                    .as_ref()
                    .map(schema_from_json_value)
                    .transpose()
                    .map_err(tool_err)?,
                mode: parse_mode(args.mode.as_deref()),
                output_format: parse_format(args.output_format.as_deref(), FormatId::Json),
                error_policy: parse_policy(args.error_policy.as_deref()),
                infer: InferMode::Conservative,
            },
            &self.state.config,
        )
        .map_err(tool_err)?;
        pack_data(result, FormatId::Json, &self.state.config)
    }

    #[tool(
        name = "shapeport_validate",
        description = "Validate data against a schema, or validate a plan"
    )]
    fn validate(
        &self,
        Parameters(args): Parameters<ValidateArgs>,
    ) -> Result<Json<ValidateOut>, McpError> {
        let result = validate_data(
            &ValidateRequest {
                source: args.source.as_ref().map(to_source).transpose()?,
                schema: args
                    .schema
                    .as_ref()
                    .map(schema_from_json_value)
                    .transpose()
                    .map_err(tool_err)?,
                plan: args.plan.as_ref().map(parse_plan_value).transpose()?,
                infer: InferMode::Conservative,
            },
            &self.state.config,
        )
        .map_err(tool_err)?;
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
    fn query(&self, Parameters(args): Parameters<QueryArgs>) -> Result<Json<DataOut>, McpError> {
        let mut sources = std::collections::HashMap::new();
        if let Some(map) = args.sources {
            for (name, source) in map {
                sources.insert(name, to_source(&source)?);
            }
        }
        if let Some(source) = args.source {
            sources.insert("input".into(), to_source(&source)?);
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
        .map_err(tool_err)?;
        pack_data(result, format, &self.state.config)
    }

    #[tool(
        name = "shapeport_convert",
        description = "Convert representation with minimal reshaping"
    )]
    fn convert(
        &self,
        Parameters(args): Parameters<ConvertArgs>,
    ) -> Result<Json<DataOut>, McpError> {
        let to = FormatId::parse(&args.to)
            .ok_or_else(|| McpError::invalid_params("unknown output format", None))?;
        let result = convert_data(
            &ConvertRequest {
                source: to_source(&args.source)?,
                to,
                infer: InferMode::Conservative,
            },
            &self.state.config,
        )
        .map_err(tool_err)?;
        pack_data(result, to, &self.state.config)
    }
}

#[tool_handler(
    name = "shapeport",
    version = "0.1.0",
    instructions = "Schema-driven data transformation runtime. Prefer inspect → plan → transform. Large results return shapeport-artifact:// URIs."
)]
impl ServerHandler for ShapePortMcp {}

fn emit_schema(schema: shapeport_core::Schema, dialect: &str) -> SchemaOut {
    let fingerprint = shapeport_core::schema_fingerprint(&schema);
    let rendered = if dialect == "json-schema" {
        schema_as_json(&schema)
    } else {
        serde_json::to_value(&schema).unwrap_or(JsonValue::Null)
    };
    SchemaOut {
        schema: rendered,
        fingerprint,
        dialect: dialect.into(),
        diagnostics: Vec::new(),
    }
}

fn pack_data(
    result: shapeport_core::TransformResult,
    format: FormatId,
    config: &RuntimeConfig,
) -> Result<Json<DataOut>, McpError> {
    let rows = result.records.len() as u64;
    let bytes = result.bytes.len() as u64;
    if bytes > config.mcp.inline_max_bytes || rows > config.mcp.inline_max_rows {
        let digest = hex::encode(Sha256::digest(&result.bytes));
        let dir = config
            .filesystem
            .write_roots
            .first()
            .cloned()
            .unwrap_or_else(|| std::path::PathBuf::from(".shapeport"));
        let path = dir.join("artifacts").join(&digest);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| McpError::internal_error(err.to_string(), None))?;
        }
        std::fs::write(&path, &result.bytes)
            .map_err(|err| McpError::internal_error(err.to_string(), None))?;
        return Ok(Json(DataOut {
            status: "ok".into(),
            result: None,
            artifact: Some(ArtifactOut {
                uri: format!("shapeport-artifact://{digest}"),
                format: format.as_str().into(),
                bytes,
                rows,
                sha256: digest,
            }),
            diagnostics: Vec::new(),
        }));
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
        diagnostics: Vec::new(),
    }))
}

fn to_source(src: &SourceRef) -> Result<SourceSpec, McpError> {
    Ok(SourceSpec {
        uri: src.uri.clone(),
        format: src.format.as_deref().and_then(FormatId::parse),
        inline: src
            .inline
            .as_ref()
            .map(|value| serde_json::from_value::<Value>(value.clone()))
            .transpose()
            .map_err(|err| McpError::invalid_params(err.to_string(), None))?,
        bytes: None,
    })
}

fn parse_plan_value(value: &JsonValue) -> Result<TransformationPlan, McpError> {
    let raw = serde_json::to_string(value)
        .map_err(|err| McpError::invalid_params(err.to_string(), None))?;
    parse_plan_json(&raw).map_err(tool_err)
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

fn tool_err(err: shapeport_core::Error) -> McpError {
    McpError::invalid_params(err.to_string(), None)
}

/// Serve MCP over stdio. stdout is protocol-only.
pub async fn serve_stdio(config: RuntimeConfig) -> Result<(), String> {
    let server = ShapePortMcp::new(config);
    let running = server.serve(stdio()).await.map_err(|err| err.to_string())?;
    running.waiting().await.map_err(|err| err.to_string())?;
    Ok(())
}

/// Serve MCP over stateless Streamable HTTP.
pub async fn serve_http(bind: SocketAddr, config: RuntimeConfig) -> Result<(), String> {
    let require_auth = !bind.ip().is_loopback();
    let token = config.mcp.bearer_token.clone();
    if require_auth && token.is_none() {
        return Err("non-loopback bind requires SHAPEPORT_MCP_TOKEN".into());
    }
    let origins = config.mcp.origin_allowlist.clone();
    let shared = Arc::new(config.clone());
    let http_config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true);
    let service = StreamableHttpService::new(
        {
            let shared = Arc::clone(&shared);
            move || Ok(ShapePortMcp::new((*shared).clone()))
        },
        LocalSessionManager::default().into(),
        http_config,
    );
    let auth_state = AuthState {
        require_auth,
        token,
        origins,
    };
    let router = Router::new()
        .nest_service("/mcp", service)
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(auth_state, auth_origin));
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|err| err.to_string())?;
    axum::serve(listener, router)
        .await
        .map_err(|err| err.to_string())
}

#[derive(Clone)]
struct AuthState {
    require_auth: bool,
    token: Option<String>,
    origins: Vec<String>,
}

async fn auth_origin(
    axum::extract::State(state): axum::extract::State<AuthState>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if let Some(origin) = headers.get(axum::http::header::ORIGIN) {
        if !origin_allowed(origin, &state.origins) {
            return Err(StatusCode::FORBIDDEN);
        }
    } else if !state.origins.is_empty() {
        return Err(StatusCode::FORBIDDEN);
    }
    if state.require_auth {
        let expected = state.token.as_deref().ok_or(StatusCode::UNAUTHORIZED)?;
        let header = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let provided = header
            .strip_prefix("Bearer ")
            .ok_or(StatusCode::UNAUTHORIZED)?;
        if provided != expected {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    Ok(next.run(request).await)
}

fn origin_allowed(origin: &HeaderValue, allow: &[String]) -> bool {
    if allow.is_empty() {
        return true;
    }
    origin
        .to_str()
        .ok()
        .is_some_and(|value| allow.iter().any(|item| item == value))
}
