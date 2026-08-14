//! RFC acceptance tests exercising public shapeport_core APIs end-to-end.

use std::collections::HashMap;
use std::path::PathBuf;

use shapeport_core::config::{InferMode, RuntimeConfig};
use shapeport_core::formats::FormatId;
use shapeport_core::{
    ConvertRequest, ErrorKind, InspectRequest, PlanRequest, PlannerMode, QueryRequest, SourceSpec,
    TransformRequest, Value, convert_data, inspect_source, plan_mapping, schema_from_json_value,
    transform_data,
};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures")
}

fn permissive_config() -> RuntimeConfig {
    RuntimeConfig::default()
}

fn file_source(path: PathBuf, format: Option<FormatId>) -> SourceSpec {
    SourceSpec {
        uri: Some(path.to_string_lossy().into_owned()),
        inline: None,
        format,
        bytes: None,
    }
}

fn bytes_source(data: Vec<u8>, format: FormatId) -> SourceSpec {
    SourceSpec {
        uri: None,
        inline: None,
        format: Some(format),
        bytes: Some(data),
    }
}

// ---------------------------------------------------------------------------
// Flint golden: inspect → plan → transform
// ---------------------------------------------------------------------------

#[test]
fn flint_inspect() {
    let config = permissive_config();
    let input = fixtures_dir().join("flint/input.json");
    let req = InspectRequest {
        source: file_source(input, Some(FormatId::Json)),
        infer: InferMode::Conservative,
        sample_rows: 10,
    };
    let result = inspect_source(&req, &config).expect("inspect failed");
    assert_eq!(result.row_count, 1);
}

#[test]
fn flint_plan_and_transform_revenue_is_string() {
    let config = permissive_config();
    let input = fixtures_dir().join("flint/input.json");
    let schema_path = fixtures_dir().join("flint/target.schema.json");

    let schema_bytes = std::fs::read(&schema_path).expect("schema file missing");
    let schema_value: serde_json::Value =
        serde_json::from_slice(&schema_bytes).expect("schema json parse failed");
    let target = schema_from_json_value(&schema_value).expect("schema_from_json_value failed");

    let input_bytes = std::fs::read(&input).expect("input file missing");
    let input_source = bytes_source(input_bytes.clone(), FormatId::Json);

    let plan_resp = plan_mapping(
        &PlanRequest {
            source_schema: None,
            target_schema: target.clone(),
            source: Some(input_source),
            mode: PlannerMode::Smart,
            infer: InferMode::Conservative,
        },
        &config,
    )
    .expect("plan_mapping failed");

    assert_eq!(plan_resp.status, "ready", "plan should be ready");
    let plan = plan_resp.plan.expect("plan missing");

    let result = transform_data(
        &TransformRequest {
            source: bytes_source(input_bytes, FormatId::Json),
            plan: Some(plan),
            target_schema: Some(target),
            mode: PlannerMode::Smart,
            output_format: FormatId::Json,
            error_policy: shapeport_core::plan::ErrorPolicy::Fail,
            infer: InferMode::Conservative,
        },
        &config,
    )
    .expect("transform_data failed");

    assert_eq!(result.records.len(), 1);
    let record = &result.records[0];
    let obj = record.as_object().expect("record should be object");
    let revenue = obj.get("revenue").expect("revenue field missing");
    assert!(
        matches!(revenue, Value::String(_)),
        "revenue must be a string, got: {revenue:?}"
    );
    assert_eq!(
        revenue.as_str().unwrap(),
        "12340.20",
        "revenue value mismatch"
    );

    // Also verify against the expected.json golden file
    let expected_bytes =
        std::fs::read(fixtures_dir().join("flint/expected.json")).expect("expected.json missing");
    let expected: Vec<serde_json::Value> =
        serde_json::from_slice(&expected_bytes).expect("expected.json parse failed");
    let actual_json: serde_json::Value =
        serde_json::from_slice(&result.bytes).expect("output json parse failed");
    assert_eq!(actual_json, serde_json::Value::Array(expected));
}

// ---------------------------------------------------------------------------
// CSV leading zeros: id stays "001"
// ---------------------------------------------------------------------------

#[test]
fn csv_leading_zeros_conservative() {
    let config = permissive_config();
    let csv_path = fixtures_dir().join("csv/people.csv");
    let req = InspectRequest {
        source: file_source(csv_path.clone(), Some(FormatId::Csv)),
        infer: InferMode::Conservative,
        sample_rows: 10,
    };
    let result = inspect_source(&req, &config).expect("inspect failed");
    assert_eq!(result.row_count, 1);

    // With conservative inference, "001" must not be coerced to integer
    let sample = &result.sample[0];
    let id = sample
        .as_object()
        .and_then(|o| o.get("id"))
        .expect("id field missing");
    assert!(
        matches!(id, Value::String(_)),
        "id should remain a string, got: {id:?}"
    );
    assert_eq!(id.as_str().unwrap(), "001");
}

// ---------------------------------------------------------------------------
// CSV → JSON convert_data
// ---------------------------------------------------------------------------

#[test]
fn csv_to_json_convert() {
    let config = permissive_config();
    let csv_path = fixtures_dir().join("csv/people.csv");
    let result = convert_data(
        &ConvertRequest {
            source: file_source(csv_path, Some(FormatId::Csv)),
            to: FormatId::Json,
            infer: InferMode::Conservative,
        },
        &config,
    )
    .expect("convert_data failed");

    let parsed: serde_json::Value =
        serde_json::from_slice(&result.bytes).expect("output not valid json");
    let arr = parsed.as_array().expect("expected array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], serde_json::Value::String("001".into()));
    assert_eq!(arr[0]["name"], serde_json::Value::String("Ada".into()));
}

// ---------------------------------------------------------------------------
// Ambiguous mapping: gross_amount + net_amount → amount
// ---------------------------------------------------------------------------

#[test]
fn ambiguous_mapping_detected() {
    let config = permissive_config();
    let source_schema_json = serde_json::json!({
        "root": {
            "kind": "record",
            "fields": [
                { "name": "gross_amount", "type": { "kind": "string" }, "nullable": false },
                { "name": "net_amount", "type": { "kind": "string" }, "nullable": false }
            ]
        }
    });
    let target_schema_json = serde_json::json!({
        "root": {
            "kind": "record",
            "fields": [
                { "name": "amount", "type": { "kind": "string" }, "nullable": false }
            ]
        }
    });
    let source_schema =
        schema_from_json_value(&source_schema_json).expect("source schema parse failed");
    let target_schema =
        schema_from_json_value(&target_schema_json).expect("target schema parse failed");

    let plan_resp = plan_mapping(
        &PlanRequest {
            source_schema: Some(source_schema),
            target_schema,
            source: None,
            mode: PlannerMode::Smart,
            infer: InferMode::Conservative,
        },
        &config,
    )
    .expect("plan_mapping should not error for ambiguous case");

    assert_eq!(
        plan_resp.status, "ambiguous",
        "expected ambiguous, got: {}",
        plan_resp.status
    );
}

// ---------------------------------------------------------------------------
// Resource limit: max_rows = 1, input has 2 rows → ResourceLimit error
// ---------------------------------------------------------------------------

#[test]
fn resource_limit_max_rows() {
    let mut config = permissive_config();
    config.limits.max_rows = 1;

    let data = serde_json::json!([
        { "x": 1 },
        { "x": 2 }
    ]);
    let bytes = serde_json::to_vec(&data).unwrap();

    let req = InspectRequest {
        source: bytes_source(bytes, FormatId::Json),
        infer: InferMode::Conservative,
        sample_rows: 10,
    };
    let err = inspect_source(&req, &config).expect_err("should have hit row limit");
    assert_eq!(
        err.kind,
        ErrorKind::ResourceLimit,
        "wrong error kind: {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Identity JSON roundtrip: JSON → JSONL → JSON
// ---------------------------------------------------------------------------

#[test]
fn identity_json_roundtrip() {
    let config = permissive_config();
    let original = serde_json::json!([{ "a": "hello", "b": 42 }]);
    let original_bytes = serde_json::to_vec(&original).unwrap();

    // JSON → JSONL
    let jsonl_result = convert_data(
        &ConvertRequest {
            source: bytes_source(original_bytes, FormatId::Json),
            to: FormatId::Jsonl,
            infer: InferMode::Conservative,
        },
        &config,
    )
    .expect("JSON→JSONL convert failed");

    // JSONL → JSON
    let json_result = convert_data(
        &ConvertRequest {
            source: bytes_source(jsonl_result.bytes, FormatId::Jsonl),
            to: FormatId::Json,
            infer: InferMode::Conservative,
        },
        &config,
    )
    .expect("JSONL→JSON convert failed");

    let roundtripped: serde_json::Value =
        serde_json::from_slice(&json_result.bytes).expect("roundtripped not valid JSON");
    assert_eq!(roundtripped, original);
}

// ---------------------------------------------------------------------------
// Query: SELECT + GROUP BY over inline sources
// ---------------------------------------------------------------------------

#[test]
fn query_simple_select() {
    let config = permissive_config();
    let data = serde_json::json!([
        { "category": "A", "amount": 10 },
        { "category": "B", "amount": 20 },
        { "category": "A", "amount": 5 }
    ]);
    let bytes = serde_json::to_vec(&data).unwrap();

    let mut sources = HashMap::new();
    sources.insert("sales".to_owned(), bytes_source(bytes, FormatId::Json));

    let result = shapeport_core::query_sources(
        &QueryRequest {
            sql: "SELECT category FROM sales ORDER BY category".to_owned(),
            sources,
            output_format: FormatId::Json,
            infer: InferMode::Conservative,
        },
        &config,
    )
    .expect("query_sources failed");

    let parsed: serde_json::Value =
        serde_json::from_slice(&result.bytes).expect("query output not valid json");
    let arr = parsed.as_array().expect("expected array");
    assert!(!arr.is_empty(), "query should return rows");
    assert!(arr[0].get("category").is_some(), "category field expected");
}
