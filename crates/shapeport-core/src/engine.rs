//! Document VM executor.

use indexmap::IndexMap;

use crate::diagnostics::Diagnostic;
use crate::error::{Error, Result};
use crate::functions;
use crate::path::{FieldPath, PathSegment};
use crate::plan::{
    CastPolicy, ErrorPolicy, Expr, NullsOrder, Operation, SortKey, SortOrder, TransformationPlan,
};
use crate::schema::Type;
use crate::value::{DecimalValue, Value};

/// A record that was dropped due to a recoverable error under `Collect` policy.
#[derive(Clone, Debug)]
pub struct RejectedRecord {
    /// Zero-based position of the record within the operation that rejected it.
    pub index: u64,
    pub diagnostic: Diagnostic,
}

/// Full output of a plan execution, including any rejected records.
#[derive(Clone, Debug)]
pub struct ExecuteOutcome {
    pub records: Vec<Value>,
    pub rejects: Vec<RejectedRecord>,
}

/// Execute a plan and return only the passing records (compatibility shim).
pub fn execute_plan(plan: &TransformationPlan, records: Vec<Value>) -> Result<Vec<Value>> {
    Ok(execute_detailed(plan, records)?.records)
}

/// Execute a plan and return both passing records and rejected records.
pub fn execute_detailed(plan: &TransformationPlan, records: Vec<Value>) -> Result<ExecuteOutcome> {
    plan.validate_shape()?;
    let mut rejects = Vec::new();
    let mut current = records;
    for op in &plan.operations {
        current = apply_op(op, current, plan.execution.error_policy, &mut rejects)?;
    }
    Ok(ExecuteOutcome {
        records: current,
        rejects,
    })
}

fn apply_op(
    op: &Operation,
    records: Vec<Value>,
    policy: ErrorPolicy,
    rejects: &mut Vec<RejectedRecord>,
) -> Result<Vec<Value>> {
    match op {
        Operation::Filter { predicate } => filter_records(predicate, records, policy, rejects),
        Operation::Sort { keys } => sort_records(keys, records),
        Operation::Explode { field, to, outer } => {
            explode_records(field, to.as_deref(), *outer, records)
        }
        other => map_records(other, records, policy, rejects),
    }
}

fn map_records(
    op: &Operation,
    records: Vec<Value>,
    policy: ErrorPolicy,
    rejects: &mut Vec<RejectedRecord>,
) -> Result<Vec<Value>> {
    let mut out = Vec::with_capacity(records.len());
    for (i, record) in records.into_iter().enumerate() {
        handle_record_result(
            apply_record(op, record),
            i as u64,
            policy,
            &mut out,
            rejects,
        )?;
    }
    Ok(out)
}

fn handle_record_result(
    result: Result<Value>,
    index: u64,
    policy: ErrorPolicy,
    out: &mut Vec<Value>,
    rejects: &mut Vec<RejectedRecord>,
) -> Result<()> {
    match result {
        Ok(value) => out.push(value),
        Err(_) if policy == ErrorPolicy::Skip => {}
        Err(err) if policy == ErrorPolicy::Collect => {
            rejects.push(RejectedRecord {
                index,
                diagnostic: Diagnostic::error(err.code, err.message),
            });
        }
        Err(err) => return Err(err),
    }
    Ok(())
}

fn apply_record(op: &Operation, record: Value) -> Result<Value> {
    match op {
        Operation::Project { fields } => project(&record, fields),
        Operation::Rename { mapping } => rename(record, mapping),
        Operation::Drop { fields } => drop_fields(record, fields),
        Operation::Literal { field, value } => insert_field(record, field, value.clone()),
        Operation::Cast {
            field,
            target,
            policy,
        } => cast_field(record, field, target, *policy),
        Operation::Coalesce { field, values } => {
            let resolved = eval_coalesce(&record, values)?;
            insert_field(record, field, resolved)
        }
        Operation::Object { fields } | Operation::Map { fields } => eval_object(&record, fields),
        Operation::Filter { .. } | Operation::Sort { .. } | Operation::Explode { .. } => {
            Err(Error::internal("cardinality op applied as record map"))
        }
    }
}

fn project(record: &Value, fields: &[FieldPath]) -> Result<Value> {
    let mut map = IndexMap::new();
    for path in fields {
        let name = path
            .simple_name()
            .ok_or_else(|| Error::plan("project_path", "project fields must be simple names"))?;
        map.insert(name.to_string(), read_path(record, path)?);
    }
    Ok(Value::Object(map))
}

fn rename(mut record: Value, mapping: &IndexMap<String, String>) -> Result<Value> {
    let object = record
        .as_object_mut()
        .ok_or_else(|| Error::transform("not_an_object", "rename requires an object record"))?;
    for (from, to) in mapping {
        if let Some(value) = object.shift_remove(from) {
            object.insert(to.clone(), value);
        }
    }
    Ok(record)
}

fn drop_fields(mut record: Value, fields: &[FieldPath]) -> Result<Value> {
    let object = record
        .as_object_mut()
        .ok_or_else(|| Error::transform("not_an_object", "drop requires an object record"))?;
    for path in fields {
        if let Some(name) = path.simple_name() {
            object.shift_remove(name);
        }
    }
    Ok(record)
}

fn insert_field(mut record: Value, field: &str, value: Value) -> Result<Value> {
    let object = record.as_object_mut().ok_or_else(|| {
        Error::transform(
            "not_an_object",
            "literal/coalesce requires an object record",
        )
    })?;
    object.insert(field.to_string(), value);
    Ok(record)
}

fn cast_field(
    mut record: Value,
    field: &FieldPath,
    target: &str,
    policy: CastPolicy,
) -> Result<Value> {
    let current = read_path(&record, field)?;
    let casted = cast_value(&current, target, policy)?;
    write_simple(&mut record, field, casted)?;
    Ok(record)
}

fn write_simple(record: &mut Value, field: &FieldPath, value: Value) -> Result<()> {
    let name = field
        .simple_name()
        .ok_or_else(|| Error::plan("cast_path", "cast field must be a simple name in v0.1"))?;
    let object = record
        .as_object_mut()
        .ok_or_else(|| Error::transform("not_an_object", "cast requires an object record"))?;
    object.insert(name.to_string(), value);
    Ok(())
}

fn eval_object(record: &Value, fields: &IndexMap<String, Expr>) -> Result<Value> {
    let mut map = IndexMap::new();
    for (name, expr) in fields {
        map.insert(name.clone(), eval_expr(record, expr)?);
    }
    Ok(Value::Object(map))
}

fn eval_coalesce(record: &Value, values: &[Expr]) -> Result<Value> {
    for expr in values {
        let value = eval_expr(record, expr)?;
        if !value.is_null() {
            return Ok(value);
        }
    }
    Ok(Value::Null)
}

fn filter_records(
    predicate: &Expr,
    records: Vec<Value>,
    policy: ErrorPolicy,
    rejects: &mut Vec<RejectedRecord>,
) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for (i, record) in records.into_iter().enumerate() {
        if eval_filter_record(&record, predicate, i as u64, policy, rejects)? {
            out.push(record);
        }
    }
    Ok(out)
}

fn eval_filter_record(
    record: &Value,
    predicate: &Expr,
    index: u64,
    policy: ErrorPolicy,
    rejects: &mut Vec<RejectedRecord>,
) -> Result<bool> {
    match eval_expr(record, predicate) {
        Ok(value) => Ok(value.is_truthy()),
        Err(_) if policy == ErrorPolicy::Skip => Ok(false),
        Err(err) if policy == ErrorPolicy::Collect => {
            rejects.push(RejectedRecord {
                index,
                diagnostic: Diagnostic::error(err.code, err.message),
            });
            Ok(false)
        }
        Err(err) => Err(err),
    }
}

fn sort_records(keys: &[SortKey], mut records: Vec<Value>) -> Result<Vec<Value>> {
    if keys.is_empty() {
        return Err(Error::plan("invalid_sort", "sort.keys must not be empty"));
    }
    records.sort_by(|left, right| compare_records(left, right, keys));
    Ok(records)
}

fn compare_records(left: &Value, right: &Value, keys: &[SortKey]) -> std::cmp::Ordering {
    for key in keys {
        let lv = read_path(left, &key.field).unwrap_or(Value::Null);
        let rv = read_path(right, &key.field).unwrap_or(Value::Null);
        let ord = compare_values(&lv, &rv, key.nulls);
        if ord != std::cmp::Ordering::Equal {
            return if key.order == SortOrder::Desc {
                ord.reverse()
            } else {
                ord
            };
        }
    }
    std::cmp::Ordering::Equal
}

fn compare_values(left: &Value, right: &Value, nulls: NullsOrder) -> std::cmp::Ordering {
    match (left.is_null(), right.is_null()) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => {
            if nulls == NullsOrder::First {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        }
        (false, true) => {
            if nulls == NullsOrder::First {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            }
        }
        (false, false) => rank_value(left).cmp(&rank_value(right)),
    }
}

fn rank_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Int(v) => format!("{v:020}"),
        Value::UInt(v) => format!("{v:020}"),
        Value::Float(v) => format!("{v:020.10}"),
        Value::Bool(v) => v.to_string(),
        Value::Decimal(v) => v.to_canonical_string(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn explode_records(
    field: &FieldPath,
    to: Option<&str>,
    outer: bool,
    records: Vec<Value>,
) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for record in records {
        explode_one(&record, field, to, outer, &mut out)?;
    }
    Ok(out)
}

fn explode_one(
    record: &Value,
    field: &FieldPath,
    to: Option<&str>,
    outer: bool,
    out: &mut Vec<Value>,
) -> Result<()> {
    match read_path(record, field)? {
        Value::Array(items) if items.is_empty() => Ok(()),
        Value::Array(items) => {
            for item in items {
                out.push(with_exploded(record, field, to, item)?);
            }
            Ok(())
        }
        Value::Null if outer => {
            out.push(with_exploded(record, field, to, Value::Null)?);
            Ok(())
        }
        Value::Null => Ok(()),
        _ => Err(Error::transform(
            "explode_type",
            "explode field must be an array",
        )),
    }
}

fn with_exploded(
    record: &Value,
    field: &FieldPath,
    to: Option<&str>,
    item: Value,
) -> Result<Value> {
    let clone = record.clone();
    let dest = to
        .map(ToString::to_string)
        .or_else(|| field.simple_name().map(ToString::to_string))
        .ok_or_else(|| Error::plan("explode_to", "explode requires a simple field or `to`"))?;
    insert_field(clone, &dest, item)
}

fn eval_expr(record: &Value, expr: &Expr) -> Result<Value> {
    match expr {
        Expr::Field(path) => read_path(record, path),
        Expr::Literal(value) => Ok(value.clone()),
        Expr::Cast {
            expr,
            target,
            policy,
        } => {
            let value = eval_expr(record, expr)?;
            cast_value(&value, target, *policy)
        }
        Expr::Coalesce(items) => eval_coalesce(record, items),
        Expr::Call { function, args } => {
            let mut values = Vec::new();
            for arg in args {
                values.push(eval_expr(record, arg)?);
            }
            functions::call(function, &values)
        }
        Expr::Object(fields) => eval_object(record, fields),
        Expr::ListMap { input, item } => eval_list_map(record, input, item),
    }
}

fn eval_list_map(record: &Value, input: &FieldPath, item_expr: &Expr) -> Result<Value> {
    match read_path(record, input)? {
        Value::Null => Ok(Value::Null),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(eval_expr(&item, item_expr)?);
            }
            Ok(Value::Array(out))
        }
        _ => Err(Error::transform(
            "list_map_type",
            "listMap input must be an array or null",
        )),
    }
}

pub fn read_path(record: &Value, path: &FieldPath) -> Result<Value> {
    let mut current = record;
    for segment in path.segments() {
        current = match (segment, current) {
            (PathSegment::Field(name), Value::Object(map)) => map.get(name).unwrap_or(&Value::Null),
            (PathSegment::Index(index), Value::Array(items)) => {
                items.get(*index).unwrap_or(&Value::Null)
            }
            _ => return Ok(Value::Null),
        };
    }
    Ok(current.clone())
}

#[allow(clippy::cast_precision_loss)]
fn cast_value(value: &Value, target: &str, policy: CastPolicy) -> Result<Value> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    let ty = Type::from_target_name(target)?;
    match (&ty, value) {
        (Type::String, Value::String(text)) => Ok(Value::String(text.clone())),
        (Type::String, other) => Ok(Value::String(stringify_value(other))),
        (Type::Int { .. }, Value::Int(v)) => Ok(Value::Int(*v)),
        (Type::Int { .. }, Value::String(text)) => parse_int(text, policy),
        (Type::Float { .. }, Value::Float(v)) => Ok(Value::Float(*v)),
        (Type::Float { .. }, Value::Int(v)) => Ok(Value::Float(*v as f64)),
        (Type::Float { .. }, Value::Decimal(_)) if policy != CastPolicy::Lossy => Err(Error::plan(
            "lossy_cast_required",
            "decimal→float64 requires policy: lossy",
        )),
        (Type::Float { .. }, Value::Decimal(dec)) => Ok(Value::Float(
            dec.to_canonical_string().parse().unwrap_or(f64::NAN),
        )),
        (Type::Float { .. }, Value::String(text)) if policy == CastPolicy::Lossy => text
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|_| Error::transform("cast_failed", format!("cannot parse float {text}"))),
        (Type::Float { .. }, Value::String(text)) if policy == CastPolicy::Try => {
            Ok(text.parse::<f64>().map_or(Value::Null, Value::Float))
        }
        (Type::Float { .. }, Value::String(_)) => Err(Error::transform(
            "lossy_cast_required",
            "string→float64 requires policy: lossy",
        )),
        (Type::Decimal { .. }, Value::Decimal(dec)) => Ok(Value::Decimal(dec.clone())),
        (Type::Decimal { .. }, Value::String(text)) => DecimalValue::parse_str(text)
            .map(Value::Decimal)
            .ok_or_else(|| Error::transform("cast_failed", format!("not a decimal: {text}"))),
        (Type::Bool, Value::Bool(v)) => Ok(Value::Bool(*v)),
        _ if policy == CastPolicy::Try => Ok(Value::Null),
        _ => Err(Error::transform(
            "cast_failed",
            format!("cannot cast to {target}"),
        )),
    }
}

fn parse_int(text: &str, policy: CastPolicy) -> Result<Value> {
    if policy == CastPolicy::Strict && text.starts_with('0') && text.len() > 1 {
        return Err(Error::transform(
            "leading_zero",
            "strict int cast refuses leading zeros",
        ));
    }
    text.parse::<i64>()
        .map(Value::Int)
        .map_err(|_| Error::transform("cast_failed", format!("not an int: {text}")))
}

fn stringify_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Decimal(dec) => dec.to_canonical_string(),
        Value::Int(v) => v.to_string(),
        Value::UInt(v) => v.to_string(),
        Value::Float(v) => v.to_string(),
        Value::Bool(v) => v.to_string(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::{execute_detailed, execute_plan};
    use crate::path::FieldPath;
    use crate::plan::{CastPolicy, ErrorPolicy, Expr, Operation, TransformationPlan};
    use crate::value::{DecimalValue, Value};

    fn decimal_record(field: &str, raw: &str) -> Value {
        Value::object([(
            field.into(),
            Value::Decimal(DecimalValue::parse_str(raw).expect("decimal")),
        )])
    }

    fn cast_plan(target: &str, policy: CastPolicy) -> TransformationPlan {
        TransformationPlan::new(vec![Operation::Cast {
            field: FieldPath::parse("x").expect("path"),
            target: target.into(),
            policy,
        }])
    }

    #[test]
    fn map_renames_fields() {
        let mut fields = IndexMap::new();
        fields.insert(
            "month".into(),
            Expr::Field(FieldPath::parse("period").expect("path")),
        );
        let plan = TransformationPlan::new(vec![Operation::Map { fields }]);
        let input = vec![Value::object([(
            "period".into(),
            Value::String("2026-01".into()),
        )])];
        let out = execute_plan(&plan, input).expect("exec");
        assert_eq!(
            out[0].as_object().expect("obj").get("month"),
            Some(&Value::String("2026-01".into()))
        );
    }

    #[test]
    fn list_map_preserves_cardinality_and_order() {
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
        let input = Value::object([(
            "customers".into(),
            Value::Array(vec![
                Value::object([("first_name".into(), Value::String("Ada".into()))]),
                Value::object([("first_name".into(), Value::String("Grace".into()))]),
            ]),
        )]);
        let out = execute_plan(&plan, vec![input]).expect("exec");
        let customers = out[0]
            .as_object()
            .expect("obj")
            .get("customers")
            .expect("customers");
        let Value::Array(items) = customers else {
            panic!("expected array");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].as_object().expect("item").get("firstName"),
            Some(&Value::String("Ada".into()))
        );
        assert_eq!(
            items[1].as_object().expect("item").get("firstName"),
            Some(&Value::String("Grace".into()))
        );
    }

    #[test]
    fn list_map_preserves_null_list() {
        let mut fields = IndexMap::new();
        fields.insert(
            "items".into(),
            Expr::ListMap {
                input: FieldPath::parse("items").expect("path"),
                item: Box::new(Expr::Field(FieldPath::parse("value").expect("path"))),
            },
        );
        let plan = TransformationPlan::new(vec![Operation::Map { fields }]);
        let input = Value::object([("items".into(), Value::Null)]);
        let out = execute_plan(&plan, vec![input]).expect("exec");
        assert_eq!(
            out[0].as_object().expect("obj").get("items"),
            Some(&Value::Null)
        );
    }

    #[test]
    fn decimal_to_float64_strict_fails() {
        let plan = cast_plan("float64", CastPolicy::Strict);
        let err = execute_plan(&plan, vec![decimal_record("x", "3.14")]).unwrap_err();
        assert_eq!(err.code, "lossy_cast_required");
    }

    #[test]
    fn decimal_to_float64_lossy_succeeds() {
        let plan = cast_plan("float64", CastPolicy::Lossy);
        let out = execute_plan(&plan, vec![decimal_record("x", "3.14")]).expect("exec");
        assert!(matches!(
            out[0].as_object().unwrap().get("x"),
            Some(Value::Float(_))
        ));
    }

    #[test]
    fn string_to_float64_strict_fails() {
        let plan = cast_plan("float64", CastPolicy::Strict);
        let record = Value::object([("x".into(), Value::String("12340.20".into()))]);
        let err = execute_plan(&plan, vec![record]).unwrap_err();
        assert_eq!(err.code, "lossy_cast_required");
    }

    #[test]
    fn collect_policy_on_bad_cast_collects_reject_and_returns_good_records() {
        let bad = Value::object([("x".into(), Value::Bool(true))]);
        let good = Value::object([("x".into(), Value::Int(42))]);
        let mut plan = cast_plan("int64", CastPolicy::Strict);
        plan.execution.error_policy = ErrorPolicy::Collect;

        let outcome = execute_detailed(&plan, vec![bad, good]).expect("exec");
        assert_eq!(outcome.records.len(), 1, "only the good record passes through");
        assert_eq!(outcome.rejects.len(), 1, "bad record must be collected");
        assert_eq!(outcome.rejects[0].index, 0);
        assert!(
            !outcome.rejects[0].diagnostic.code.is_empty(),
            "reject must carry a diagnostic code"
        );
    }

    #[test]
    fn flint_map_keeps_revenue_as_string() {
        let mut fields = IndexMap::new();
        fields.insert(
            "month".into(),
            Expr::Field(FieldPath::parse("period").expect("path")),
        );
        fields.insert(
            "product".into(),
            Expr::Field(FieldPath::parse("product_family").expect("path")),
        );
        fields.insert(
            "revenue".into(),
            Expr::Field(FieldPath::parse("total_sales_usd").expect("path")),
        );
        let plan = TransformationPlan::new(vec![Operation::Map { fields }]);
        let record = Value::object([
            ("period".into(), Value::String("2026-01".into())),
            ("product_family".into(), Value::String("electronics".into())),
            ("total_sales_usd".into(), Value::String("12340.20".into())),
        ]);
        let out = execute_plan(&plan, vec![record]).expect("exec");
        let obj = out[0].as_object().expect("obj");
        assert_eq!(obj.get("month"), Some(&Value::String("2026-01".into())));
        assert_eq!(
            obj.get("product"),
            Some(&Value::String("electronics".into()))
        );
        assert_eq!(obj.get("revenue"), Some(&Value::String("12340.20".into())));
    }
}
