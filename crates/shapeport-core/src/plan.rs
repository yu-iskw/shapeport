//! Transformation Plan IR (`shapeport.dev/v1alpha1`).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::path::FieldPath;
use crate::schema::Type;
use crate::value::Value;

pub const API_VERSION: &str = "shapeport.dev/v1alpha1";
pub const KIND: &str = "TransformationPlan";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CastPolicy {
    Strict,
    Lossy,
    Try,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorPolicy {
    Fail,
    Skip,
    Collect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortOrder {
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NullsOrder {
    First,
    Last,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::derive_partial_eq_without_eq)]
pub enum Expr {
    Field(FieldPath),
    Literal(Value),
    Cast {
        expr: Box<Self>,
        target: String,
        policy: CastPolicy,
    },
    Coalesce(Vec<Self>),
    Call {
        function: String,
        args: Vec<Self>,
    },
    Object(IndexMap<String, Self>),
    ListMap {
        input: FieldPath,
        item: Box<Self>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortKey {
    pub field: FieldPath,
    #[serde(default = "default_asc")]
    pub order: SortOrder,
    #[serde(default = "default_nulls_last")]
    pub nulls: NullsOrder,
}

const fn default_asc() -> SortOrder {
    SortOrder::Asc
}

const fn default_nulls_last() -> NullsOrder {
    NullsOrder::Last
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Operation {
    Project {
        fields: Vec<FieldPath>,
    },
    Rename {
        mapping: IndexMap<String, String>,
    },
    Drop {
        fields: Vec<FieldPath>,
    },
    Literal {
        field: String,
        value: Value,
    },
    Cast {
        field: FieldPath,
        target: String,
        policy: CastPolicy,
    },
    Coalesce {
        field: String,
        values: Vec<Expr>,
    },
    Object {
        fields: IndexMap<String, Expr>,
    },
    Map {
        fields: IndexMap<String, Expr>,
    },
    Filter {
        predicate: Expr,
    },
    Sort {
        keys: Vec<SortKey>,
    },
    Explode {
        field: FieldPath,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
        #[serde(default)]
        outer: bool,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_by: Option<GeneratedBy>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedBy {
    pub mode: String,
    pub shapeport_version: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContractRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Contracts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<ContractRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<ContractRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSpec {
    #[serde(default = "default_fail")]
    pub error_policy: ErrorPolicy,
    #[serde(default = "default_backend")]
    pub backend: String,
}

const fn default_fail() -> ErrorPolicy {
    ErrorPolicy::Fail
}

fn default_backend() -> String {
    "document".into()
}

impl Default for ExecutionSpec {
    fn default() -> Self {
        Self {
            error_policy: ErrorPolicy::Fail,
            backend: default_backend(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformationPlan {
    pub api_version: String,
    pub kind: String,
    #[serde(default)]
    pub metadata: PlanMetadata,
    #[serde(default)]
    pub contracts: Contracts,
    pub operations: Vec<Operation>,
    #[serde(default)]
    pub validation: ValidationSpec,
    #[serde(default)]
    pub execution: ExecutionSpec,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidationSpec {
    #[serde(default = "default_output_required")]
    pub output: String,
}

fn default_output_required() -> String {
    "required".into()
}

impl Default for ValidationSpec {
    fn default() -> Self {
        Self {
            output: default_output_required(),
        }
    }
}

impl TransformationPlan {
    #[must_use]
    pub fn new(operations: Vec<Operation>) -> Self {
        Self {
            api_version: API_VERSION.into(),
            kind: KIND.into(),
            metadata: PlanMetadata::default(),
            contracts: Contracts::default(),
            operations,
            validation: ValidationSpec::default(),
            execution: ExecutionSpec::default(),
        }
    }

    pub fn validate_shape(&self) -> Result<()> {
        if self.api_version != API_VERSION {
            return Err(Error::plan(
                "unsupported_api_version",
                format!("unsupported apiVersion {}", self.api_version),
            ));
        }
        if self.kind != KIND {
            return Err(Error::plan("invalid_kind", format!("kind must be {KIND}")));
        }
        if self.operations.is_empty() {
            return Err(Error::plan("empty_plan", "operations must not be empty"));
        }
        if self.execution.backend != "document" {
            return Err(Error::plan(
                "unsupported_backend",
                format!("backend must be document, got {}", self.execution.backend),
            ));
        }
        validate_ops(&self.operations)
    }
}

fn validate_ops(ops: &[Operation]) -> Result<()> {
    for op in ops {
        validate_op(op)?;
    }
    Ok(())
}

fn validate_op(op: &Operation) -> Result<()> {
    match op {
        Operation::Sort { keys } if keys.is_empty() => {
            Err(Error::plan("invalid_sort", "sort.keys must not be empty"))
        }
        Operation::Map { fields } if fields.is_empty() => {
            Err(Error::plan("invalid_map", "map.fields must not be empty"))
        }
        Operation::Project { fields } if fields.is_empty() => Err(Error::plan(
            "invalid_project",
            "project.fields must not be empty",
        )),
        Operation::Cast { target, policy, .. } => validate_cast_target(target, *policy),
        Operation::Map { fields } | Operation::Object { fields } => validate_fields_exprs(fields),
        Operation::Coalesce { values, .. } => validate_exprs(values),
        Operation::Filter { predicate } => validate_expr(predicate),
        _ => Ok(()),
    }
}

fn validate_cast_target(target: &str, policy: CastPolicy) -> Result<()> {
    let ty = Type::from_target_name(target)?;
    if matches!(ty, Type::Float { .. }) && policy == CastPolicy::Strict {
        return Err(Error::plan(
            "lossy_cast_required",
            "decimal/string→float64 requires policy: lossy",
        ));
    }
    Ok(())
}

fn validate_expr(expr: &Expr) -> Result<()> {
    match expr {
        Expr::Cast {
            target,
            policy,
            expr: inner,
        } => {
            validate_cast_target(target, *policy)?;
            validate_expr(inner)
        }
        Expr::Coalesce(items) => validate_exprs(items),
        Expr::Call { args, .. } => validate_exprs(args),
        Expr::Object(fields) => validate_fields_exprs(fields),
        Expr::ListMap { item, .. } => validate_expr(item),
        Expr::Field(_) | Expr::Literal(_) => Ok(()),
    }
}

fn validate_exprs(exprs: &[Expr]) -> Result<()> {
    for expr in exprs {
        validate_expr(expr)?;
    }
    Ok(())
}

fn validate_fields_exprs(fields: &IndexMap<String, Expr>) -> Result<()> {
    for expr in fields.values() {
        validate_expr(expr)?;
    }
    Ok(())
}

pub fn parse_plan_json(raw: &str) -> Result<TransformationPlan> {
    let plan: TransformationPlan = serde_json::from_str(raw)?;
    plan.validate_shape()?;
    Ok(plan)
}

pub fn parse_plan_bytes(raw: &[u8], as_yaml: bool) -> Result<TransformationPlan> {
    if as_yaml {
        let value: serde_json::Value = serde_norway::from_slice(raw)
            .map_err(|err| Error::parse("yaml_plan", err.to_string()))?;
        let plan: TransformationPlan = serde_json::from_value(value)?;
        plan.validate_shape()?;
        return Ok(plan);
    }
    parse_plan_json(
        std::str::from_utf8(raw).map_err(|err| Error::parse("plan_encoding", err.to_string()))?,
    )
}

#[cfg(test)]
mod tests {
    use super::{CastPolicy, Expr, Operation, TransformationPlan, parse_plan_json};
    use crate::path::FieldPath;
    use indexmap::IndexMap;

    #[test]
    fn rejects_jsonpath_in_plan() {
        let raw = r#"{"apiVersion":"shapeport.dev/v1alpha1","kind":"TransformationPlan","operations":[{"map":{"fields":{"month":{"field":"$.period"}}}}]}"#;
        assert!(parse_plan_json(raw).is_err());
    }

    #[test]
    fn accepts_normative_map_plan() {
        let mut fields = IndexMap::new();
        fields.insert(
            "month".into(),
            Expr::Field(FieldPath::parse("period").expect("path")),
        );
        let plan = TransformationPlan::new(vec![Operation::Map { fields }]);
        let json = serde_json::to_string(&plan).expect("json");
        let parsed = parse_plan_json(&json).expect("parse");
        assert_eq!(parsed.operations.len(), 1);
    }

    #[test]
    fn accepts_list_map_expression() {
        let mut item_fields = IndexMap::new();
        item_fields.insert(
            "firstName".into(),
            Expr::Field(FieldPath::parse("first_name").expect("path")),
        );
        let mut fields = IndexMap::new();
        fields.insert(
            "customers".into(),
            Expr::ListMap {
                input: FieldPath::parse("customers").expect("path"),
                item: Box::new(Expr::Object(item_fields)),
            },
        );
        let plan = TransformationPlan::new(vec![Operation::Map { fields }]);
        let json = serde_json::to_string(&plan).expect("json");
        let parsed = parse_plan_json(&json).expect("parse");
        assert_eq!(parsed.operations.len(), 1);
    }

    #[test]
    fn strict_float_cast_fails_validation() {
        let raw = r#"{"apiVersion":"shapeport.dev/v1alpha1","kind":"TransformationPlan","operations":[{"cast":{"field":"x","target":"float64","policy":"strict"}}]}"#;
        let err = parse_plan_json(raw).expect_err("strict float cast must be rejected");
        assert_eq!(err.code, "lossy_cast_required");
    }

    #[test]
    fn lossy_float_cast_passes_validation() {
        let raw = r#"{"apiVersion":"shapeport.dev/v1alpha1","kind":"TransformationPlan","operations":[{"cast":{"field":"x","target":"float64","policy":"lossy"}}]}"#;
        parse_plan_json(raw).expect("lossy float cast must be accepted");
    }

    #[test]
    fn strict_float_cast_in_map_expr_fails_validation() {
        let cast_expr = Expr::Cast {
            expr: Box::new(Expr::Field(FieldPath::parse("x").expect("path"))),
            target: "float64".into(),
            policy: CastPolicy::Strict,
        };
        let mut fields = IndexMap::new();
        fields.insert("y".into(), cast_expr);
        let plan = TransformationPlan::new(vec![Operation::Map { fields }]);
        let json = serde_json::to_string(&plan).expect("json");
        let err = parse_plan_json(&json).expect_err("strict float cast in map must be rejected");
        assert_eq!(err.code, "lossy_cast_required");
    }
}
