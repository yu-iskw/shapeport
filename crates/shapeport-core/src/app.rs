//! Application services shared by CLI and MCP.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::{InferMode, RuntimeConfig};
use crate::diagnostics::Diagnostic;
use crate::engine::execute_plan;
use crate::error::{Error, Result};
use crate::fingerprint::schema_fingerprint;
use crate::formats::{
    Detection, FormatId, decode_records, detect_format, encode_records, infer_schema,
};
use crate::json_schema::{schema_from_json_schema, schema_to_json_schema, validate_value};
use crate::plan::{ErrorPolicy, TransformationPlan, parse_plan_bytes};
use crate::planner::{
    ExplainEntry, PlanOutcome, PlannerMode, PlannerOptions, Unresolved, plan_schemas,
};
use crate::query::execute_sql;
use crate::schema::Schema;
use crate::security::{read_artifact, read_limited, resolve_read_path, resolve_write_path};
use crate::value::Value;

/// A record that was rejected during execution, with its index and reason.
#[derive(Clone, Debug)]
pub struct RejectedRecord {
    /// Zero-based index of the rejected record in the input.
    pub index: usize,
    /// Diagnostic describing why the record was rejected.
    pub diagnostic: Diagnostic,
}

/// Detailed output from plan execution.
#[derive(Clone, Debug)]
pub struct ExecuteOutcome {
    /// Records that passed transformation.
    pub records: Vec<Value>,
    /// Records that were rejected (with cause).
    pub rejects: Vec<RejectedRecord>,
}

#[derive(Clone, Debug)]
pub struct SourceSpec {
    pub uri: Option<String>,
    pub inline: Option<Value>,
    pub format: Option<FormatId>,
    pub bytes: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct InspectRequest {
    pub source: SourceSpec,
    pub infer: InferMode,
    pub sample_rows: usize,
}

#[derive(Clone, Debug)]
pub struct InspectResult {
    pub detection: Detection,
    pub schema: Schema,
    pub fingerprint: String,
    pub row_count: usize,
    pub sample: Vec<Value>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn inspect_source(request: &InspectRequest, config: &RuntimeConfig) -> Result<InspectResult> {
    let (bytes, path) = load_source(&request.source, config)?;
    let detection = detect_format(&bytes, path.as_deref(), request.source.format);
    let records = decode_records(&bytes, detection.format, request.infer)?;
    check_row_limit(records.len(), config)?;
    let schema = infer_schema(&records, request.infer)?;
    let sample_rows = request.sample_rows.min(records.len());
    Ok(InspectResult {
        fingerprint: schema_fingerprint(&schema),
        schema,
        detection,
        row_count: records.len(),
        sample: records.into_iter().take(sample_rows).collect(),
        diagnostics: Vec::new(),
    })
}

#[derive(Clone, Debug)]
pub struct PlanRequest {
    pub source_schema: Option<Schema>,
    pub target_schema: Schema,
    pub source: Option<SourceSpec>,
    pub mode: PlannerMode,
    pub infer: InferMode,
}

#[derive(Clone, Debug)]
pub struct PlanResponse {
    pub status: String,
    pub plan: Option<TransformationPlan>,
    pub explanation: Vec<ExplainEntry>,
    pub unresolved: Vec<Unresolved>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn plan_mapping(request: &PlanRequest, config: &RuntimeConfig) -> Result<PlanResponse> {
    let source_schema = if let Some(schema) = &request.source_schema {
        schema.clone()
    } else {
        let source = request
            .source
            .as_ref()
            .ok_or_else(|| Error::usage("plan_source", "plan requires sourceSchema or source"))?;
        inspect_source(
            &InspectRequest {
                source: source.clone(),
                infer: request.infer,
                sample_rows: config.sample_rows,
            },
            config,
        )?
        .schema
    };
    let options = PlannerOptions {
        mode: request.mode,
        config: config.planner.clone(),
    };
    match plan_schemas(&source_schema, &request.target_schema, &options)? {
        PlanOutcome::Ready {
            plan,
            explanation,
            diagnostics,
        } => Ok(PlanResponse {
            status: "ready".into(),
            plan: Some(plan),
            explanation,
            unresolved: Vec::new(),
            diagnostics,
        }),
        PlanOutcome::Ambiguous {
            unresolved,
            explanation,
            diagnostics,
        } => Ok(PlanResponse {
            status: "ambiguous".into(),
            plan: None,
            explanation,
            unresolved,
            diagnostics,
        }),
    }
}

#[derive(Clone, Debug)]
pub struct TransformRequest {
    pub source: SourceSpec,
    pub plan: Option<TransformationPlan>,
    pub target_schema: Option<Schema>,
    pub mode: PlannerMode,
    pub output_format: FormatId,
    pub error_policy: ErrorPolicy,
    pub infer: InferMode,
}

#[derive(Clone, Debug)]
pub struct TransformResult {
    pub records: Vec<Value>,
    pub bytes: Vec<u8>,
    pub schema: Option<Schema>,
    pub diagnostics: Vec<Diagnostic>,
    pub rejects: Vec<RejectedRecord>,
}

pub fn transform_data(
    request: &TransformRequest,
    config: &RuntimeConfig,
) -> Result<TransformResult> {
    let (bytes, path) = load_source(&request.source, config)?;
    let detection = detect_format(&bytes, path.as_deref(), request.source.format);
    let records = decode_records(&bytes, detection.format, request.infer)?;
    check_row_limit(records.len(), config)?;
    check_nesting_depth(&records, config)?;
    let plan = resolve_plan(request, &records, config)?;
    let mut plan = plan;
    plan.execution.error_policy = request.error_policy;
    // Use execute_plan as the fallback since execute_detailed is not yet in engine.
    let out = execute_plan(&plan, records)?;
    if let Some(schema) = &request.target_schema
        && plan.validation.output != "none"
    {
        for record in &out {
            validate_value(schema, record)?;
        }
    }
    let encoded = encode_records(&out, request.output_format)?;
    if encoded.len() as u64 > config.limits.max_output_bytes {
        return Err(Error::limit(
            "max_output_bytes",
            "encoded output exceeds maxOutputBytes",
        ));
    }
    Ok(TransformResult {
        records: out,
        bytes: encoded,
        schema: request.target_schema.clone(),
        diagnostics: Vec::new(),
        rejects: Vec::new(),
    })
}

fn resolve_plan(
    request: &TransformRequest,
    records: &[Value],
    config: &RuntimeConfig,
) -> Result<TransformationPlan> {
    if let Some(plan) = &request.plan {
        plan.validate_shape()?;
        return Ok(plan.clone());
    }
    let target = request
        .target_schema
        .as_ref()
        .ok_or_else(|| Error::usage("transform_plan", "transform requires plan or targetSchema"))?;
    let source_schema = infer_schema(records, request.infer)?;
    let planned = plan_mapping(
        &PlanRequest {
            source_schema: Some(source_schema),
            target_schema: target.clone(),
            source: None,
            mode: request.mode,
            infer: request.infer,
        },
        config,
    )?;
    planned
        .plan
        .ok_or_else(|| Error::ambiguity("mapping is ambiguous"))
}

#[derive(Clone, Debug)]
pub struct ValidateRequest {
    pub source: Option<SourceSpec>,
    pub schema: Option<Schema>,
    pub plan: Option<TransformationPlan>,
    pub infer: InferMode,
}

#[derive(Clone, Debug)]
pub struct ValidateResult {
    pub valid: bool,
    pub errors: Vec<Diagnostic>,
}

pub fn validate_data(request: &ValidateRequest, config: &RuntimeConfig) -> Result<ValidateResult> {
    if let Some(plan) = &request.plan {
        return match plan.validate_shape() {
            Ok(()) => Ok(ValidateResult {
                valid: true,
                errors: Vec::new(),
            }),
            Err(err) => Ok(ValidateResult {
                valid: false,
                errors: vec![Diagnostic::error(err.code.clone(), err.message)],
            }),
        };
    }
    let source = request.source.as_ref().ok_or_else(|| {
        Error::usage("validate_source", "validate requires source+schema or plan")
    })?;
    let schema = request
        .schema
        .as_ref()
        .ok_or_else(|| Error::usage("validate_schema", "validate requires a schema"))?;
    let (bytes, path) = load_source(source, config)?;
    let detection = detect_format(&bytes, path.as_deref(), source.format);
    let records = decode_records(&bytes, detection.format, request.infer)?;
    let mut errors = Vec::new();
    for (idx, record) in records.iter().enumerate() {
        if let Err(err) = validate_value(schema, record) {
            errors.push(Diagnostic::error(
                err.code,
                format!("record {idx}: {}", err.message),
            ));
        }
    }
    Ok(ValidateResult {
        valid: errors.is_empty(),
        errors,
    })
}

#[derive(Clone, Debug)]
pub struct QueryRequest {
    pub sql: String,
    pub sources: HashMap<String, SourceSpec>,
    pub output_format: FormatId,
    pub infer: InferMode,
}

pub fn query_sources(request: &QueryRequest, config: &RuntimeConfig) -> Result<TransformResult> {
    let mut tables = HashMap::new();
    for (name, source) in &request.sources {
        let (bytes, path) = load_source(source, config)?;
        let detection = detect_format(&bytes, path.as_deref(), source.format);
        let records = decode_records(&bytes, detection.format, request.infer)?;
        check_row_limit(records.len(), config)?;
        tables.insert(name.clone(), records);
    }
    if tables.len() == 1
        && let Some((_, rows)) = tables.iter().next()
    {
        tables.insert("input".into(), rows.clone());
    }
    let records = execute_sql(&request.sql, &tables)?;
    let bytes = encode_records(&records, request.output_format)?;
    Ok(TransformResult {
        records,
        bytes,
        schema: None,
        diagnostics: Vec::new(),
        rejects: Vec::new(),
    })
}

#[derive(Clone, Debug)]
pub struct ConvertRequest {
    pub source: SourceSpec,
    pub to: FormatId,
    pub infer: InferMode,
}

pub fn convert_data(request: &ConvertRequest, config: &RuntimeConfig) -> Result<TransformResult> {
    let (bytes, path) = load_source(&request.source, config)?;
    let detection = detect_format(&bytes, path.as_deref(), request.source.format);
    let records = decode_records(&bytes, detection.format, request.infer)?;
    check_row_limit(records.len(), config)?;
    let encoded = encode_records(&records, request.to)?;
    Ok(TransformResult {
        records,
        bytes: encoded,
        schema: None,
        diagnostics: Vec::new(),
        rejects: Vec::new(),
    })
}

pub fn schema_from_json_value(value: &serde_json::Value) -> Result<Schema> {
    schema_from_json_schema(value)
}

#[must_use]
pub fn schema_as_json(schema: &Schema) -> serde_json::Value {
    schema_to_json_schema(schema)
}

pub fn load_plan_file(path: &Path, config: &RuntimeConfig) -> Result<TransformationPlan> {
    let resolved = resolve_read_path(&path.to_string_lossy(), config)?;
    let bytes = read_limited(&resolved, config.limits.max_input_bytes)?;
    let yaml = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "yaml" || ext == "yml");
    parse_plan_bytes(&bytes, yaml)
}

pub fn write_output(path: &Path, bytes: &[u8], config: &RuntimeConfig) -> Result<()> {
    let resolved = resolve_write_path(&path.to_string_lossy(), config)?;
    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(resolved, bytes)?;
    Ok(())
}

fn load_source(source: &SourceSpec, config: &RuntimeConfig) -> Result<(Vec<u8>, Option<PathBuf>)> {
    if let Some(value) = &source.inline {
        return Ok((serde_json::to_vec(value)?, None));
    }
    if let Some(bytes) = &source.bytes {
        if bytes.len() as u64 > config.limits.max_input_bytes {
            return Err(Error::limit("max_input_bytes", "inline bytes exceed limit"));
        }
        return Ok((bytes.clone(), None));
    }
    let uri = source
        .uri
        .as_deref()
        .ok_or_else(|| Error::usage("missing_source", "source uri or inline data is required"))?;
    if uri == "-" {
        return load_stdin(config);
    }
    if uri.starts_with("shapeport-artifact://") {
        let bytes = read_artifact(uri, config)?;
        return Ok((bytes, None));
    }
    let path = resolve_read_path(uri, config)?;
    let bytes = read_limited(&path, config.limits.max_input_bytes)?;
    Ok((bytes, Some(path)))
}

fn load_stdin(config: &RuntimeConfig) -> Result<(Vec<u8>, Option<PathBuf>)> {
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)?;
    if buf.len() as u64 > config.limits.max_input_bytes {
        return Err(Error::limit("max_input_bytes", "stdin exceeds limit"));
    }
    Ok((buf, None))
}

fn check_row_limit(rows: usize, config: &RuntimeConfig) -> Result<()> {
    if rows as u64 > config.limits.max_rows {
        return Err(Error::limit("max_rows", "row count exceeds configured max"));
    }
    Ok(())
}

/// Check that no record exceeds the configured nesting depth.
fn check_nesting_depth(records: &[Value], config: &RuntimeConfig) -> Result<()> {
    let max = config.limits.max_nesting_depth;
    for record in records {
        let depth = value_depth(record);
        if depth > max {
            return Err(Error::limit(
                "max_nesting_depth",
                format!("record nesting depth {depth} exceeds max_nesting_depth {max}"),
            ));
        }
    }
    Ok(())
}

/// Compute the maximum nesting depth of a JSON-like value (1 for scalars).
fn value_depth(value: &Value) -> u32 {
    match value {
        Value::Object(map) => {
            let child_max = map.values().map(value_depth).max().unwrap_or(0);
            1 + child_max
        }
        Value::Array(items) => {
            let child_max = items.iter().map(value_depth).max().unwrap_or(0);
            1 + child_max
        }
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExecuteOutcome, InspectRequest, RejectedRecord, SourceSpec, inspect_source, value_depth,
    };
    use crate::config::{InferMode, RuntimeConfig};
    use crate::formats::FormatId;
    use crate::security::write_artifact;
    use crate::value::Value;

    #[test]
    fn inspects_inline_json() {
        let request = InspectRequest {
            source: SourceSpec {
                uri: None,
                inline: Some(Value::Array(vec![Value::object([(
                    "period".into(),
                    Value::String("2026-01".into()),
                )])])),
                format: Some(FormatId::Json),
                bytes: None,
            },
            infer: InferMode::Conservative,
            sample_rows: 10,
        };
        let result = inspect_source(&request, &RuntimeConfig::default()).expect("inspect");
        assert_eq!(result.row_count, 1);
        assert!(result.fingerprint.starts_with("sha256:"));
    }

    #[test]
    fn value_depth_scalar() {
        assert_eq!(value_depth(&Value::String("hi".into())), 1);
        assert_eq!(value_depth(&Value::Null), 1);
    }

    #[test]
    fn value_depth_nested() {
        let inner = Value::object([("x".into(), Value::Int(1))]);
        let outer = Value::object([("a".into(), inner)]);
        assert_eq!(value_depth(&outer), 3);
    }

    #[test]
    fn artifact_source_roundtrip() {
        let root = {
            let mut p = std::env::temp_dir();
            p.push(format!("shapeport_app_test_{}", std::process::id()));
            p
        };
        let mut config = RuntimeConfig::default();
        config.filesystem.write_roots = vec![root.clone()];
        config.filesystem.read_roots = vec![root.clone()];
        config.mcp.artifact_ttl_secs = 3600;

        let data = br#"[{"id":1}]"#;
        let meta = write_artifact(data, &config).expect("write");
        let uri = format!("shapeport-artifact://{}", meta.digest);

        let spec = SourceSpec {
            uri: Some(uri),
            inline: None,
            format: Some(FormatId::Json),
            bytes: None,
        };
        let req = InspectRequest {
            source: spec,
            infer: InferMode::Conservative,
            sample_rows: 5,
        };
        let result = inspect_source(&req, &config).expect("inspect");
        assert_eq!(result.row_count, 1);
    }

    #[test]
    fn rejected_record_and_execute_outcome_types_exist() {
        let rr = RejectedRecord {
            index: 0,
            diagnostic: crate::diagnostics::Diagnostic::error("e", "m"),
        };
        let _outcome = ExecuteOutcome {
            records: Vec::new(),
            rejects: vec![rr],
        };
    }
}
