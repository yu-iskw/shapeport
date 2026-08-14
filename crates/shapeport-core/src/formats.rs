//! Format detection, decoding, and encoding.

use std::io::{BufReader, Cursor};
use std::path::Path;

use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use arrow::ipc::reader::{FileReader as IpcFileReader, StreamReader as IpcStreamReader};
use arrow::ipc::writer::FileWriter as IpcFileWriter;
use arrow::json::reader::{ReaderBuilder as ArrowJsonReader, infer_json_schema};
use arrow::json::writer::LineDelimitedWriter;
use bytes::Bytes;
use indexmap::IndexMap;
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::properties::WriterProperties;

use crate::config::InferMode;
use crate::error::{Error, Result};
use crate::schema::{Field, Schema, Type};
use crate::value::{DecimalValue, Value};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatId {
    Json,
    Jsonl,
    Yaml,
    Csv,
    Tsv,
    Parquet,
    ArrowIpc,
}

impl FormatId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Jsonl => "jsonl",
            Self::Yaml => "yaml",
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Parquet => "parquet",
            Self::ArrowIpc => "arrow-ipc",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "json" => Some(Self::Json),
            "jsonl" | "ndjson" => Some(Self::Jsonl),
            "yaml" | "yml" => Some(Self::Yaml),
            "csv" => Some(Self::Csv),
            "tsv" => Some(Self::Tsv),
            "parquet" => Some(Self::Parquet),
            "arrow" | "arrow-ipc" | "ipc" => Some(Self::ArrowIpc),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Detection {
    pub format: FormatId,
    pub confidence: f64,
    pub evidence: Vec<String>,
}

/// Options controlling how CSV/TSV cells are decoded.
///
/// `null_spellings` lists cell values that decode to `Value::Null` rather than a string.
/// The default is an empty list, meaning empty cells produce `Value::String("")`.
#[derive(Clone, Debug, Default)]
pub struct DecodeOptions {
    pub infer: InferMode,
    pub null_spellings: Vec<String>,
}

impl DecodeOptions {
    #[must_use]
    pub const fn new(infer: InferMode) -> Self {
        Self {
            infer,
            null_spellings: Vec::new(),
        }
    }
}

pub fn detect_format(bytes: &[u8], path: Option<&Path>, explicit: Option<FormatId>) -> Detection {
    if let Some(format) = explicit {
        return Detection {
            format,
            confidence: 1.0,
            evidence: vec!["explicit".into()],
        };
    }
    if bytes.len() >= 4 && bytes.starts_with(b"PAR1") {
        return Detection {
            format: FormatId::Parquet,
            confidence: 1.0,
            evidence: vec!["magic:PAR1".into()],
        };
    }
    if bytes.len() >= 6 && bytes.starts_with(b"ARROW1") {
        return Detection {
            format: FormatId::ArrowIpc,
            confidence: 1.0,
            evidence: vec!["magic:ARROW1".into()],
        };
    }
    if let Some(from_ext) = path.and_then(format_from_path) {
        return Detection {
            format: from_ext,
            confidence: 0.85,
            evidence: vec![format!("extension:{}", from_ext.as_str())],
        };
    }
    probe_text(bytes)
}

fn format_from_path(path: &Path) -> Option<FormatId> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "json" => Some(FormatId::Json),
        "jsonl" | "ndjson" => Some(FormatId::Jsonl),
        "yaml" | "yml" => Some(FormatId::Yaml),
        "csv" => Some(FormatId::Csv),
        "tsv" => Some(FormatId::Tsv),
        "parquet" => Some(FormatId::Parquet),
        "arrow" | "ipc" => Some(FormatId::ArrowIpc),
        _ => None,
    }
}

fn probe_text(bytes: &[u8]) -> Detection {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
            return Detection {
                format: FormatId::Json,
                confidence: 0.95,
                evidence: vec!["utf8_text".into(), "json_parse".into()],
            };
        }
        return Detection {
            format: FormatId::Jsonl,
            confidence: 0.6,
            evidence: vec!["json_like".into()],
        };
    }
    if looks_like_jsonl(&text) {
        return Detection {
            format: FormatId::Jsonl,
            confidence: 0.9,
            evidence: vec!["jsonl_lines".into()],
        };
    }
    if trimmed.contains(':') && !trimmed.contains(',') {
        return Detection {
            format: FormatId::Yaml,
            confidence: 0.55,
            evidence: vec!["yaml_like".into()],
        };
    }
    Detection {
        format: FormatId::Csv,
        confidence: 0.5,
        evidence: vec!["delimited_fallback".into()],
    }
}

fn looks_like_jsonl(text: &str) -> bool {
    let mut seen = 0;
    for line in text.lines().take(8) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !(line.starts_with('{') && serde_json::from_str::<serde_json::Value>(line).is_ok()) {
            return false;
        }
        seen += 1;
    }
    seen > 0
}

/// Decode records using default options (no null spellings).
pub fn decode_records(bytes: &[u8], format: FormatId, infer: InferMode) -> Result<Vec<Value>> {
    decode_records_with(bytes, format, &DecodeOptions::new(infer))
}

/// Decode records with full control over null spellings and inference mode.
pub fn decode_records_with(
    bytes: &[u8],
    format: FormatId,
    options: &DecodeOptions,
) -> Result<Vec<Value>> {
    match format {
        FormatId::Json => decode_json(bytes),
        FormatId::Jsonl => decode_jsonl(bytes),
        FormatId::Yaml => decode_yaml(bytes),
        FormatId::Csv => decode_delimited(bytes, b',', options),
        FormatId::Tsv => decode_delimited(bytes, b'\t', options),
        FormatId::Parquet => decode_parquet(bytes),
        FormatId::ArrowIpc => decode_arrow_ipc(bytes),
    }
}

pub fn encode_records(records: &[Value], format: FormatId) -> Result<Vec<u8>> {
    match format {
        FormatId::Json => encode_json(records),
        FormatId::Jsonl => encode_jsonl(records),
        FormatId::Yaml => encode_yaml(records),
        FormatId::Csv => encode_delimited(records, b','),
        FormatId::Tsv => encode_delimited(records, b'\t'),
        FormatId::Parquet => encode_parquet(records),
        FormatId::ArrowIpc => encode_arrow_ipc(records),
    }
}

fn decode_json(bytes: &[u8]) -> Result<Vec<Value>> {
    let value: Value = serde_json::from_slice(bytes)?;
    Ok(records_from_value(value))
}

fn decode_jsonl(bytes: &[u8]) -> Result<Vec<Value>> {
    let mut records = Vec::new();
    for (idx, line) in String::from_utf8_lossy(bytes).lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|err| Error::parse("jsonl_line", format!("line {}: {err}", idx + 1)))?;
        records.push(as_record(value));
    }
    Ok(records)
}

fn decode_yaml(bytes: &[u8]) -> Result<Vec<Value>> {
    let json: serde_json::Value =
        serde_yml::from_slice(bytes).map_err(|err| Error::parse("yaml_error", err.to_string()))?;
    let value: Value = serde_json::from_value(json)?;
    Ok(records_from_value(value))
}

fn decode_delimited(bytes: &[u8], delimiter: u8, options: &DecodeOptions) -> Result<Vec<Value>> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(true)
        .from_reader(bytes);
    let headers: Vec<String> = reader.headers()?.iter().map(ToString::to_string).collect();
    let mut records = Vec::new();
    for row in reader.records() {
        let row = row?;
        let mut map = IndexMap::new();
        for (idx, header) in headers.iter().enumerate() {
            let cell = row.get(idx).unwrap_or("");
            map.insert(header.clone(), infer_cell(cell, options));
        }
        records.push(Value::Object(map));
    }
    Ok(records)
}

/// Map a single CSV/TSV cell to a `Value`.
///
/// Empty cells and any cell whose text exactly matches a `null_spellings` entry
/// produce `Value::Null`. All other empty cells produce `Value::String("")`.
fn infer_cell(cell: &str, options: &DecodeOptions) -> Value {
    if options.null_spellings.iter().any(|s| s.as_str() == cell) {
        return Value::Null;
    }
    match options.infer {
        InferMode::None => Value::String(cell.to_string()),
        InferMode::Conservative => conservative_cell(cell),
        InferMode::Aggressive => aggressive_cell(cell),
    }
}

/// Returns `true` when `cell` has a leading zero followed by another digit,
/// e.g. `"01"`, `"001"`, `"01.5"`. Plain `"0"` and `"0.5"` return `false`.
fn has_leading_zero_numeric(cell: &str) -> bool {
    let mut chars = cell.chars();
    chars.next() == Some('0') && chars.next().is_some_and(|c| c.is_ascii_digit())
}

fn conservative_cell(cell: &str) -> Value {
    if has_leading_zero_numeric(cell) {
        return Value::String(cell.to_string());
    }
    if let Ok(int) = cell.parse::<i64>() {
        return Value::Int(int);
    }
    if cell.contains('.')
        && let Some(dec) = DecimalValue::parse_str(cell)
    {
        return Value::Decimal(dec);
    }
    Value::String(cell.to_string())
}

/// Returns `Some(f64)` only for cells that contain a decimal point, parse as
/// a finite float, and do **not** carry a leading zero before the decimal.
fn try_parse_float(cell: &str) -> Option<f64> {
    if !cell.contains('.') || has_leading_zero_numeric(cell) {
        return None;
    }
    cell.parse::<f64>().ok()
}

fn aggressive_cell(cell: &str) -> Value {
    match cell {
        "true" | "TRUE" | "yes" => return Value::Bool(true),
        "false" | "FALSE" | "no" => return Value::Bool(false),
        _ => {}
    }
    if let Some(float) = try_parse_float(cell) {
        return Value::Float(float);
    }
    conservative_cell(cell)
}

fn decode_parquet(bytes: &[u8]) -> Result<Vec<Value>> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(Bytes::copy_from_slice(bytes))
        .map_err(|err| Error::parse("parquet_error", err.to_string()))?;
    let reader = builder
        .build()
        .map_err(|err| Error::parse("parquet_error", err.to_string()))?;
    batches_to_records(reader)
}

fn decode_arrow_ipc(bytes: &[u8]) -> Result<Vec<Value>> {
    if let Ok(reader) = IpcFileReader::try_new(Cursor::new(bytes), None) {
        return batches_to_records(reader);
    }
    let reader = IpcStreamReader::try_new(Cursor::new(bytes), None)
        .map_err(|err| Error::parse("arrow_ipc", err.to_string()))?;
    batches_to_records(reader)
}

fn batches_to_records<I>(reader: I) -> Result<Vec<Value>>
where
    I: IntoIterator<Item = std::result::Result<RecordBatch, arrow::error::ArrowError>>,
{
    let mut json_bytes = Vec::new();
    {
        let mut writer = LineDelimitedWriter::new(&mut json_bytes);
        for batch in reader {
            let batch = batch.map_err(|err| Error::parse("arrow_batch", err.to_string()))?;
            writer
                .write(&batch)
                .map_err(|err| Error::parse("arrow_json", err.to_string()))?;
        }
        writer
            .finish()
            .map_err(|err| Error::parse("arrow_json", err.to_string()))?;
    }
    decode_jsonl(&json_bytes)
}

fn encode_json(records: &[Value]) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(&Value::Array(records.to_vec()))?)
}

fn encode_jsonl(records: &[Value]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for record in records {
        out.extend(serde_json::to_vec(record)?);
        out.push(b'\n');
    }
    Ok(out)
}

fn encode_yaml(records: &[Value]) -> Result<Vec<u8>> {
    let json = serde_json::to_value(Value::Array(records.to_vec()))?;
    serde_yml::to_string(&json)
        .map(String::into_bytes)
        .map_err(|err| Error::transform("yaml_encode", err.to_string()))
}

fn encode_delimited(records: &[Value], delimiter: u8) -> Result<Vec<u8>> {
    let headers = delimited_headers(records);
    let mut writer = csv::WriterBuilder::new()
        .delimiter(delimiter)
        .from_writer(Vec::new());
    writer.write_record(&headers)?;
    for record in records {
        let object = record.as_object().ok_or_else(|| {
            Error::transform("csv_nested", "CSV/TSV output requires flat object records")
        })?;
        let mut row = Vec::new();
        for header in &headers {
            row.push(cell_text(object.get(header).unwrap_or(&Value::Null))?);
        }
        writer.write_record(&row)?;
    }
    writer
        .into_inner()
        .map_err(|err| Error::io_err(err.to_string()))
}

fn delimited_headers(records: &[Value]) -> Vec<String> {
    let mut headers = Vec::new();
    if let Some(first) = records.first().and_then(Value::as_object) {
        headers.extend(first.keys().cloned());
    }
    headers
}

fn cell_text(value: &Value) -> Result<String> {
    match value {
        Value::Null => Ok(String::new()),
        Value::String(text) => Ok(text.clone()),
        Value::Bool(flag) => Ok(flag.to_string()),
        Value::Int(v) => Ok(v.to_string()),
        Value::UInt(v) => Ok(v.to_string()),
        Value::Float(v) => Ok(v.to_string()),
        Value::Decimal(v) => Ok(v.to_canonical_string()),
        Value::Array(_) | Value::Object(_) => Err(Error::transform(
            "csv_nested",
            "nested values cannot be written to CSV/TSV in v0.1",
        )),
        other => Ok(serde_json::to_string(other).unwrap_or_default()),
    }
}

fn encode_parquet(records: &[Value]) -> Result<Vec<u8>> {
    let batch = records_to_batch(records)?;
    let mut buffer = Vec::new();
    let props = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(&mut buffer, batch.schema(), Some(props))
        .map_err(|err| Error::transform("parquet_write", err.to_string()))?;
    writer
        .write(&batch)
        .map_err(|err| Error::transform("parquet_write", err.to_string()))?;
    writer
        .close()
        .map_err(|err| Error::transform("parquet_write", err.to_string()))?;
    Ok(buffer)
}

fn encode_arrow_ipc(records: &[Value]) -> Result<Vec<u8>> {
    let batch = records_to_batch(records)?;
    let mut buffer = Vec::new();
    {
        let mut writer = IpcFileWriter::try_new(&mut buffer, &batch.schema())
            .map_err(|err| Error::transform("arrow_write", err.to_string()))?;
        writer
            .write(&batch)
            .map_err(|err| Error::transform("arrow_write", err.to_string()))?;
        writer
            .finish()
            .map_err(|err| Error::transform("arrow_write", err.to_string()))?;
    }
    Ok(buffer)
}

fn records_to_batch(records: &[Value]) -> Result<RecordBatch> {
    let jsonl = encode_jsonl(records)?;
    let schema = infer_arrow_schema(&jsonl)?;
    let cursor = Cursor::new(jsonl);
    let mut reader = ArrowJsonReader::new(SchemaRef::new(schema))
        .build(cursor)
        .map_err(|err| Error::transform("arrow_json", err.to_string()))?;
    reader
        .next()
        .transpose()
        .map_err(|err| Error::transform("arrow_json", err.to_string()))?
        .ok_or_else(|| Error::transform("arrow_json", "no record batch produced"))
}

fn infer_arrow_schema(jsonl: &[u8]) -> Result<arrow::datatypes::Schema> {
    let mut reader = BufReader::new(Cursor::new(jsonl));
    let (schema, _) = infer_json_schema(&mut reader, None)
        .map_err(|err| Error::transform("arrow_schema", err.to_string()))?;
    Ok(schema)
}

fn records_from_value(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items,
        other => vec![other],
    }
}

fn as_record(value: Value) -> Value {
    match value {
        Value::Object(_) => value,
        other => Value::object([("value".into(), other)]),
    }
}

pub fn infer_schema(records: &[Value], infer: InferMode) -> Result<Schema> {
    let Some(first) = records.first().and_then(Value::as_object) else {
        return Ok(Schema::record(Vec::new()));
    };
    let mut fields = Vec::new();
    for (name, value) in first {
        fields.push(Field::new(name, type_of(value), value.is_null()));
    }
    unify_fields(&mut fields, records, infer)?;
    Ok(Schema::record(fields))
}

fn unify_fields(fields: &mut [Field], records: &[Value], infer: InferMode) -> Result<()> {
    for record in records.iter().skip(1) {
        let Some(object) = record.as_object() else {
            continue;
        };
        for field in fields.iter_mut() {
            if let Some(value) = object.get(&field.name) {
                let next = type_of(value);
                field.ty = unify_types(&field.ty, &next, infer)?;
                if value.is_null() {
                    field.nullable = true;
                }
            } else {
                field.nullable = true;
            }
        }
    }
    Ok(())
}

fn type_of(value: &Value) -> Type {
    match value {
        Value::Null => Type::Unknown,
        Value::Bool(_) => Type::Bool,
        Value::Int(_) => Type::Int {
            bits: 64,
            signed: true,
        },
        Value::UInt(_) => Type::Int {
            bits: 64,
            signed: false,
        },
        Value::Float(_) => Type::Float { bits: 64 },
        Value::Decimal(_) => Type::Decimal {
            precision: 38,
            scale: 10,
        },
        Value::Array(items) => Type::list(items.first().map_or(Type::Any, type_of), true),
        Value::Object(map) => Type::record(
            map.iter()
                .map(|(k, v)| Field::new(k, type_of(v), v.is_null()))
                .collect(),
        ),
        _ => Type::String,
    }
}

fn unify_types(left: &Type, right: &Type, infer: InferMode) -> Result<Type> {
    if matches!(left, Type::Unknown) {
        return Ok(right.clone());
    }
    if matches!(right, Type::Unknown) || left == right {
        return Ok(left.clone());
    }
    if infer == InferMode::Conservative {
        return Err(Error::schema(
            "type_conflict",
            "conservative inference failed on mid-stream type conflict",
        ));
    }
    Ok(Type::Union {
        variants: vec![left.clone(), right.clone()],
    })
}

#[cfg(test)]
mod tests {
    use super::{
        DecodeOptions, FormatId, decode_records, decode_records_with, detect_format,
        encode_records, infer_schema,
    };
    use crate::config::InferMode;
    use crate::value::Value;

    #[test]
    fn detects_json_array() {
        let bytes = br#"[{"a":1}]"#;
        let detection = detect_format(bytes, None, None);
        assert_eq!(detection.format, FormatId::Json);
        let records =
            decode_records(bytes, FormatId::Json, InferMode::Conservative).expect("decode");
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn csv_keeps_leading_zeros() {
        let bytes = b"id,code\n1,001\n";
        let records = decode_records(bytes, FormatId::Csv, InferMode::Conservative).expect("csv");
        let obj = records[0].as_object().expect("obj");
        assert_eq!(obj.get("code"), Some(&Value::String("001".into())));
    }

    #[test]
    fn empty_csv_field_is_empty_string() {
        let bytes = b"a,b\n,x\n";
        let records = decode_records(bytes, FormatId::Csv, InferMode::Conservative).expect("csv");
        let obj = records[0].as_object().expect("obj");
        assert_eq!(obj.get("a"), Some(&Value::String(String::new())));
    }

    #[test]
    fn empty_csv_field_null_when_spelled() {
        let opts = DecodeOptions {
            infer: InferMode::Conservative,
            null_spellings: vec![String::new()],
        };
        let bytes = b"a,b\n,x\n";
        let records = decode_records_with(bytes, FormatId::Csv, &opts).expect("csv");
        let obj = records[0].as_object().expect("obj");
        assert_eq!(obj.get("a"), Some(&Value::Null));
    }

    #[test]
    fn infer_schema_type_conflict_conservative() {
        let records = vec![
            Value::object([("a".into(), Value::Int(1))]),
            Value::object([("a".into(), Value::String("abc".into()))]),
        ];
        let err = infer_schema(&records, InferMode::Conservative).unwrap_err();
        assert_eq!(err.code, "type_conflict");
    }

    #[test]
    fn aggressive_keeps_leading_zero_integer() {
        let bytes = b"code\n001\n";
        let records = decode_records(bytes, FormatId::Csv, InferMode::Aggressive).expect("csv");
        let obj = records[0].as_object().expect("obj");
        assert_eq!(obj.get("code"), Some(&Value::String("001".into())));
    }

    #[test]
    fn aggressive_keeps_leading_zero_decimal() {
        let bytes = b"val\n01.5\n";
        let records = decode_records(bytes, FormatId::Csv, InferMode::Aggressive).expect("csv");
        let obj = records[0].as_object().expect("obj");
        assert_eq!(obj.get("val"), Some(&Value::String("01.5".into())));
    }

    #[test]
    fn conservative_keeps_leading_zero_decimal() {
        let bytes = b"val\n01.5\n";
        let records = decode_records(bytes, FormatId::Csv, InferMode::Conservative).expect("csv");
        let obj = records[0].as_object().expect("obj");
        assert_eq!(obj.get("val"), Some(&Value::String("01.5".into())));
    }

    #[test]
    fn aggressive_promotes_plain_float() {
        let bytes = b"val\n1.5\n";
        let records = decode_records(bytes, FormatId::Csv, InferMode::Aggressive).expect("csv");
        let obj = records[0].as_object().expect("obj");
        assert_eq!(obj.get("val"), Some(&Value::Float(1.5)));
    }

    #[test]
    fn aggressive_promotes_bool() {
        let bytes = b"flag\ntrue\n";
        let records = decode_records(bytes, FormatId::Csv, InferMode::Aggressive).expect("csv");
        let obj = records[0].as_object().expect("obj");
        assert_eq!(obj.get("flag"), Some(&Value::Bool(true)));
    }

    #[test]
    fn csv_nested_encode_returns_error() {
        let records = vec![Value::object([
            ("a".into(), Value::Int(1)),
            ("nested".into(), Value::Array(vec![])),
        ])];
        let err = encode_records(&records, FormatId::Csv).unwrap_err();
        assert_eq!(err.code, "csv_nested");
    }
}
