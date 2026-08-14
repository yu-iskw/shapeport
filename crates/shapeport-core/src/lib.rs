//! `ShapePort` core library: schema, plans, document VM, formats, and app services.

#![forbid(unsafe_code)]

pub mod app;
pub mod config;
pub mod diagnostics;
pub mod engine;
pub mod error;
pub mod fingerprint;
pub mod formats;
pub mod functions;
pub mod json_schema;
pub mod path;
pub mod plan;
pub mod planner;
pub mod query;
pub mod schema;
pub mod security;
pub mod value;

pub use app::{
    ConvertRequest, ExecuteOutcome, InspectRequest, InspectResult, PlanRequest, PlanResponse,
    QueryRequest, RejectedRecord, SourceSpec, TransformRequest, TransformResult, ValidateRequest,
    ValidateResult, convert_data, inspect_source, plan_mapping, query_sources, schema_as_json,
    schema_from_json_value, transform_data, validate_data, write_output,
};
pub use config::RuntimeConfig;
pub use diagnostics::{Diagnostic, Severity};
pub use engine::{execute_plan, read_path};
pub use error::{Error, ErrorKind, Result};
pub use fingerprint::schema_fingerprint;
pub use formats::{FormatId, detect_format};
pub use path::FieldPath;
pub use plan::{CastPolicy, Expr, Operation, TransformationPlan};
pub use planner::{PlannerMode, PlannerOptions};
pub use schema::{Field, Schema, TimeUnit, Type};
pub use security::{ArtifactMeta, read_artifact, write_artifact};
pub use value::{DecimalValue, Value};
