//! Bounded SQL over registered in-memory record tables.

use std::cmp::Ordering;
use std::collections::HashMap;

use indexmap::IndexMap;
use sqlparser::ast::{
    BinaryOperator, Expr as SqlExpr, FunctionArg, FunctionArgExpr, FunctionArguments, GroupByExpr,
    Join, JoinConstraint, JoinOperator, LimitClause, ObjectName, OrderBy, OrderByKind,
    Query as SqlQuery, Select, SelectItem, SetExpr, Statement, TableFactor, TableWithJoins,
    Value as SqlValue, ValueWithSpan,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

use crate::error::{Error, Result};
use crate::value::Value;

pub fn execute_sql(sql: &str, tables: &HashMap<String, Vec<Value>>) -> Result<Vec<Value>> {
    let statements = Parser::parse_sql(&GenericDialect {}, sql)
        .map_err(|err| Error::parse("sql_parse", err.to_string()))?;
    let Some(Statement::Query(query)) = statements.into_iter().next() else {
        return Err(Error::usage(
            "sql_unsupported",
            "only a single SELECT statement is allowed",
        ));
    };
    execute_query(&query, tables)
}

fn execute_query(query: &SqlQuery, tables: &HashMap<String, Vec<Value>>) -> Result<Vec<Value>> {
    reject_unsupported(query)?;
    let SetExpr::Select(select) = query.body.as_ref() else {
        return Err(Error::usage("sql_unsupported", "only SELECT is supported"));
    };
    let mut rows = load_from(select, tables)?;
    if let Some(selection) = &select.selection {
        rows.retain(|row| eval_bool(selection, row).unwrap_or(false));
    }
    let mut rows = project_or_group(select, rows)?;
    if let Some(order) = query.order_by.as_ref() {
        sort_by(order, &mut rows)?;
    }
    if let Some(n) = query_limit(query) {
        rows.truncate(n);
    }
    Ok(rows)
}

fn query_limit(query: &SqlQuery) -> Option<usize> {
    match &query.limit_clause {
        Some(LimitClause::LimitOffset {
            limit: Some(expr), ..
        }) => eval_limit(expr),
        Some(LimitClause::OffsetCommaLimit { limit, .. }) => eval_limit(limit),
        _ => None,
    }
}

fn reject_unsupported(query: &SqlQuery) -> Result<()> {
    if query.with.is_some() {
        return Err(Error::usage("sql_unsupported", "WITH/CTE is not supported"));
    }
    Ok(())
}

fn load_from(select: &Select, tables: &HashMap<String, Vec<Value>>) -> Result<Vec<Value>> {
    let Some(with_joins) = select.from.first() else {
        return Err(Error::usage("sql_from", "FROM is required"));
    };
    let mut rows = table_rows(with_joins, tables)?;
    for join in &with_joins.joins {
        rows = apply_join(&rows, join, tables)?;
    }
    Ok(rows)
}

fn table_rows(from: &TableWithJoins, tables: &HashMap<String, Vec<Value>>) -> Result<Vec<Value>> {
    named_table_rows(&from.relation, tables)
}

fn named_table_rows(
    factor: &TableFactor,
    tables: &HashMap<String, Vec<Value>>,
) -> Result<Vec<Value>> {
    let TableFactor::Table { name, alias, .. } = factor else {
        return Err(Error::usage("sql_from", "only named tables are allowed"));
    };
    let key = object_name(name);
    let rows = tables.get(&key).or_else(|| {
        alias
            .as_ref()
            .and_then(|alias| tables.get(&alias.name.value))
    });
    let Some(rows) = rows.or_else(|| tables.get("input")) else {
        return Err(Error::usage(
            "sql_unknown_table",
            format!("table {key} is not a registered source"),
        ));
    };
    Ok(rows.clone())
}

fn object_name(name: &ObjectName) -> String {
    name.0
        .last()
        .and_then(sqlparser::ast::ObjectNamePart::as_ident)
        .map(|ident| ident.value.clone())
        .unwrap_or_default()
}

fn apply_join(
    left_rows: &[Value],
    join: &Join,
    tables: &HashMap<String, Vec<Value>>,
) -> Result<Vec<Value>> {
    let right = named_table_rows(&join.relation, tables)?;
    let (is_left, constraint) = join_sides(&join.join_operator)?;
    let JoinConstraint::On(expr) = constraint else {
        return Err(Error::usage("sql_join", "JOIN must use ON equality"));
    };
    Ok(equi_join(
        left_rows,
        &right,
        expr,
        is_left,
        &table_factor_name(&join.relation),
    ))
}

fn join_sides(operator: &JoinOperator) -> Result<(bool, &JoinConstraint)> {
    match operator {
        JoinOperator::Inner(constraint) | JoinOperator::Join(constraint) => Ok((false, constraint)),
        JoinOperator::LeftOuter(constraint) | JoinOperator::Left(constraint) => {
            Ok((true, constraint))
        }
        _ => Err(Error::usage(
            "sql_join",
            "only INNER JOIN and LEFT JOIN are supported",
        )),
    }
}

fn table_factor_name(factor: &TableFactor) -> String {
    match factor {
        TableFactor::Table { name, alias, .. } => alias
            .as_ref()
            .map_or_else(|| object_name(name), |alias| alias.name.value.clone()),
        _ => String::new(),
    }
}

fn equi_join(
    left_rows: &[Value],
    right_rows: &[Value],
    on: &SqlExpr,
    is_left: bool,
    right_name: &str,
) -> Vec<Value> {
    let mut out = Vec::new();
    for left in left_rows {
        let mut matched = false;
        for right in right_rows {
            if eval_join_bool(on, left, right, right_name).unwrap_or(false) {
                out.push(merge_objects(left, right));
                matched = true;
            }
        }
        if is_left && !matched {
            out.push(left.clone());
        }
    }
    out
}

fn eval_join_bool(expr: &SqlExpr, left: &Value, right: &Value, right_name: &str) -> Result<bool> {
    match eval_join_expr(expr, left, right, right_name)? {
        Value::Bool(flag) => Ok(flag),
        Value::Null => Ok(false),
        other => Ok(other.is_truthy()),
    }
}

fn eval_join_expr(expr: &SqlExpr, left: &Value, right: &Value, right_name: &str) -> Result<Value> {
    match expr {
        SqlExpr::Identifier(ident) => Ok(lookup_unqualified(&ident.value, left, right)),
        SqlExpr::CompoundIdentifier(parts) => Ok(lookup_qualified(parts, left, right, right_name)),
        SqlExpr::Value(value) => literal(&value.value),
        SqlExpr::BinaryOp {
            left: lhs,
            op,
            right: rhs,
        } => {
            let lv = eval_join_expr(lhs, left, right, right_name)?;
            let rv = eval_join_expr(rhs, left, right, right_name)?;
            eval_binary_values(op, &lv, &rv)
        }
        _ => Err(Error::usage(
            "sql_expr",
            format!("unsupported SQL expression {expr}"),
        )),
    }
}

fn lookup_qualified(
    parts: &[sqlparser::ast::Ident],
    left: &Value,
    right: &Value,
    right_name: &str,
) -> Value {
    let Some(col) = parts.last() else {
        return Value::Null;
    };
    if parts.len() >= 2 && parts[parts.len() - 2].value == right_name {
        lookup(right, &col.value)
    } else {
        lookup(left, &col.value)
    }
}

fn lookup_unqualified(name: &str, left: &Value, right: &Value) -> Value {
    if left.as_object().is_some_and(|map| map.contains_key(name)) {
        lookup(left, name)
    } else {
        lookup(right, name)
    }
}

fn merge_objects(left: &Value, right: &Value) -> Value {
    let mut map = left.as_object().cloned().unwrap_or_default();
    if let Some(right) = right.as_object() {
        for (k, v) in right {
            map.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    Value::Object(map)
}

fn project_or_group(select: &Select, rows: Vec<Value>) -> Result<Vec<Value>> {
    if is_grouped(select) {
        return group_by(select, rows);
    }
    let mut out = Vec::new();
    for row in rows {
        out.push(project_row(select, &row)?);
    }
    Ok(out)
}

fn is_grouped(select: &Select) -> bool {
    match &select.group_by {
        GroupByExpr::Expressions(exprs, _) => !exprs.is_empty(),
        GroupByExpr::All(_) => select.projection.iter().any(is_agg_item),
    }
}

fn is_agg_item(item: &SelectItem) -> bool {
    matches!(item, SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } if is_agg_expr(expr))
}

fn is_agg_expr(expr: &SqlExpr) -> bool {
    matches!(expr, SqlExpr::Function(func) if ["count", "sum", "avg", "min", "max"]
        .contains(&func.name.to_string().to_ascii_lowercase().as_str()))
}

fn group_by(select: &Select, rows: Vec<Value>) -> Result<Vec<Value>> {
    let keys = group_key_exprs(select);
    let mut groups: IndexMap<String, Vec<Value>> = IndexMap::new();
    for row in rows {
        let key = group_key(&keys, &row)?;
        groups.entry(key).or_default().push(row);
    }
    let mut out = Vec::new();
    for members in groups.into_values() {
        let Some(first) = members.first() else {
            continue;
        };
        out.push(project_group(select, first, &members)?);
    }
    Ok(out)
}

fn group_key_exprs(select: &Select) -> Vec<SqlExpr> {
    match &select.group_by {
        GroupByExpr::Expressions(exprs, _) => exprs.clone(),
        GroupByExpr::All(_) => Vec::new(),
    }
}

fn group_key(keys: &[SqlExpr], row: &Value) -> Result<String> {
    let mut parts = Vec::new();
    for key in keys {
        parts.push(serde_json::to_string(&eval_expr(key, row)?)?);
    }
    Ok(parts.join("|"))
}

fn project_row(select: &Select, row: &Value) -> Result<Value> {
    if matches!(select.projection.as_slice(), [SelectItem::Wildcard(_)]) {
        return Ok(row.clone());
    }
    let mut map = IndexMap::new();
    for item in &select.projection {
        match item {
            SelectItem::UnnamedExpr(expr) => {
                let name = expr_name(expr);
                map.insert(name, eval_expr(expr, row)?);
            }
            SelectItem::ExprWithAlias { expr, alias } => {
                map.insert(alias.value.clone(), eval_expr(expr, row)?);
            }
            SelectItem::Wildcard(_) => {
                if let Some(object) = row.as_object() {
                    for (k, v) in object {
                        map.insert(k.clone(), v.clone());
                    }
                }
            }
            SelectItem::QualifiedWildcard(_, _) => {
                return Err(Error::usage("sql_select", "unsupported select item"));
            }
        }
    }
    Ok(Value::Object(map))
}

fn project_group(select: &Select, first: &Value, members: &[Value]) -> Result<Value> {
    let mut map = IndexMap::new();
    for item in &select.projection {
        match item {
            SelectItem::UnnamedExpr(expr) => {
                map.insert(expr_name(expr), eval_group_expr(expr, first, members)?);
            }
            SelectItem::ExprWithAlias { expr, alias } => {
                map.insert(alias.value.clone(), eval_group_expr(expr, first, members)?);
            }
            SelectItem::QualifiedWildcard(_, _) | SelectItem::Wildcard(_) => {
                return Err(Error::usage(
                    "sql_select",
                    "unsupported grouped select item",
                ));
            }
        }
    }
    Ok(Value::Object(map))
}

fn eval_group_expr(expr: &SqlExpr, first: &Value, members: &[Value]) -> Result<Value> {
    if is_agg_expr(expr) {
        return eval_agg(expr, members);
    }
    eval_expr(expr, first)
}

fn eval_agg(expr: &SqlExpr, members: &[Value]) -> Result<Value> {
    let SqlExpr::Function(func) = expr else {
        return Err(Error::internal("expected aggregate function"));
    };
    let name = func.name.to_string().to_ascii_lowercase();
    let arg = first_arg(func);
    match name.as_str() {
        "count" => Ok(Value::Int(i64::try_from(members.len()).unwrap_or(i64::MAX))),
        "sum" => Ok(Value::Float(fold_nums(members, arg, 0.0, |a, b| a + b)?)),
        "avg" => {
            let sum = fold_nums(members, arg, 0.0, |a, b| a + b)?;
            Ok(Value::Float(sum / members.len().max(1) as f64))
        }
        "min" => Ok(Value::Float(fold_nums(
            members,
            arg,
            f64::INFINITY,
            f64::min,
        )?)),
        "max" => Ok(Value::Float(fold_nums(
            members,
            arg,
            f64::NEG_INFINITY,
            f64::max,
        )?)),
        _ => Err(Error::usage(
            "sql_udf",
            format!("unsupported function {name}"),
        )),
    }
}

fn first_arg(func: &sqlparser::ast::Function) -> Option<&SqlExpr> {
    let FunctionArguments::List(list) = &func.args else {
        return None;
    };
    match list.args.first() {
        Some(FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))) => Some(expr),
        _ => None,
    }
}

fn fold_nums(
    members: &[Value],
    arg: Option<&SqlExpr>,
    init: f64,
    fold: fn(f64, f64) -> f64,
) -> Result<f64> {
    let mut acc = init;
    for member in members {
        let value = match arg {
            Some(expr) => eval_expr(expr, member)?,
            None => Value::Int(1),
        };
        acc = fold(acc, value_f64(&value)?);
    }
    Ok(acc)
}

fn eval_bool(expr: &SqlExpr, row: &Value) -> Result<bool> {
    match eval_expr(expr, row)? {
        Value::Bool(flag) => Ok(flag),
        Value::Null => Ok(false),
        other => Ok(other.is_truthy()),
    }
}

fn eval_expr(expr: &SqlExpr, row: &Value) -> Result<Value> {
    match expr {
        SqlExpr::Identifier(ident) => Ok(lookup(row, &ident.value)),
        SqlExpr::CompoundIdentifier(parts) => Ok(lookup(row, &parts.last().expect("ident").value)),
        SqlExpr::Value(value) => literal(&value.value),
        SqlExpr::BinaryOp { left, op, right } => eval_binary(left, op, right, row),
        SqlExpr::Function(_) if is_agg_expr(expr) => Err(Error::usage(
            "sql_agg",
            "aggregate functions require GROUP BY or grouped projection",
        )),
        _ => Err(Error::usage(
            "sql_expr",
            format!("unsupported SQL expression {expr}"),
        )),
    }
}

fn eval_binary(left: &SqlExpr, op: &BinaryOperator, right: &SqlExpr, row: &Value) -> Result<Value> {
    let lv = eval_expr(left, row)?;
    let rv = eval_expr(right, row)?;
    eval_binary_values(op, &lv, &rv)
}

fn eval_binary_values(op: &BinaryOperator, left: &Value, right: &Value) -> Result<Value> {
    match op {
        BinaryOperator::Eq
        | BinaryOperator::NotEq
        | BinaryOperator::Gt
        | BinaryOperator::Lt
        | BinaryOperator::GtEq
        | BinaryOperator::LtEq => Ok(Value::Bool(eval_compare(op, left, right))),
        BinaryOperator::And | BinaryOperator::Or => Ok(Value::Bool(eval_logic(op, left, right))),
        BinaryOperator::Plus
        | BinaryOperator::Minus
        | BinaryOperator::Multiply
        | BinaryOperator::Divide => eval_arith(op, left, right),
        _ => Err(Error::usage("sql_op", format!("unsupported operator {op}"))),
    }
}

fn eval_compare(op: &BinaryOperator, left: &Value, right: &Value) -> bool {
    match op {
        BinaryOperator::Eq => values_equal(left, right),
        BinaryOperator::NotEq => !values_equal(left, right),
        BinaryOperator::Gt => values_cmp(left, right) == Ordering::Greater,
        BinaryOperator::Lt => values_cmp(left, right) == Ordering::Less,
        BinaryOperator::GtEq => values_cmp(left, right) != Ordering::Less,
        BinaryOperator::LtEq => values_cmp(left, right) != Ordering::Greater,
        _ => unreachable!("eval_compare called with non-comparison operator: {op}"),
    }
}

fn eval_logic(op: &BinaryOperator, left: &Value, right: &Value) -> bool {
    match op {
        BinaryOperator::And => left.is_truthy() && right.is_truthy(),
        BinaryOperator::Or => left.is_truthy() || right.is_truthy(),
        _ => unreachable!("eval_logic called with non-logical operator: {op}"),
    }
}

fn eval_arith(op: &BinaryOperator, left: &Value, right: &Value) -> Result<Value> {
    let left_n = value_f64(left)?;
    let right_n = value_f64(right)?;
    let result = match op {
        BinaryOperator::Plus => left_n + right_n,
        BinaryOperator::Minus => left_n - right_n,
        BinaryOperator::Multiply => left_n * right_n,
        BinaryOperator::Divide => left_n / right_n,
        _ => unreachable!("eval_arith called with non-arithmetic operator: {op}"),
    };
    Ok(Value::Float(result))
}

fn literal(value: &SqlValue) -> Result<Value> {
    match value {
        SqlValue::Number(raw, _) => {
            if raw.contains('.') {
                Ok(Value::Float(raw.parse().unwrap_or(0.0)))
            } else {
                Ok(Value::Int(raw.parse().unwrap_or(0)))
            }
        }
        SqlValue::SingleQuotedString(text) | SqlValue::DoubleQuotedString(text) => {
            Ok(Value::String(text.clone()))
        }
        SqlValue::Boolean(flag) => Ok(Value::Bool(*flag)),
        SqlValue::Null => Ok(Value::Null),
        _ => Err(Error::usage("sql_literal", "unsupported literal")),
    }
}

fn lookup(row: &Value, name: &str) -> Value {
    row.as_object()
        .and_then(|map| map.get(name).cloned())
        .unwrap_or(Value::Null)
}

fn expr_name(expr: &SqlExpr) -> String {
    match expr {
        SqlExpr::Identifier(ident) => ident.value.clone(),
        SqlExpr::CompoundIdentifier(parts) => {
            parts.last().map(|p| p.value.clone()).unwrap_or_default()
        }
        SqlExpr::Function(func) => func.name.to_string().to_ascii_lowercase(),
        other => other.to_string(),
    }
}

fn values_equal(left: &Value, right: &Value) -> bool {
    left == right || stringify(left) == stringify(right)
}

fn stringify(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Int(v) => v.to_string(),
        Value::UInt(v) => v.to_string(),
        Value::Float(v) => v.to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Decimal(v) => v.to_canonical_string(),
        _ => String::new(),
    }
}

fn numeric_rank(value: &Value) -> Option<f64> {
    match value {
        Value::Int(v) => Some(i64_to_f64(*v)),
        Value::UInt(v) => Some(*v as f64),
        Value::Float(v) => Some(*v),
        Value::Decimal(dec) => dec.to_canonical_string().parse().ok(),
        _ => None,
    }
}

#[allow(clippy::cast_precision_loss)]
fn i64_to_f64(value: i64) -> f64 {
    value as f64
}

fn values_cmp(left: &Value, right: &Value) -> Ordering {
    match (numeric_rank(left), numeric_rank(right)) {
        (Some(left_n), Some(right_n)) => left_n.total_cmp(&right_n),
        _ => stringify(left).cmp(&stringify(right)),
    }
}

fn value_f64(value: &Value) -> Result<f64> {
    match value {
        Value::Int(v) => Ok(*v as f64),
        Value::UInt(v) => Ok(*v as f64),
        Value::Float(v) => Ok(*v),
        Value::String(text) => text
            .parse()
            .map_err(|_| Error::transform("sql_num", format!("not numeric: {text}"))),
        Value::Decimal(dec) => dec
            .to_canonical_string()
            .parse()
            .map_err(|_| Error::transform("sql_num", "decimal is not finite")),
        _ => Ok(0.0),
    }
}

fn eval_limit(expr: &SqlExpr) -> Option<usize> {
    match expr {
        SqlExpr::Value(ValueWithSpan {
            value: SqlValue::Number(raw, _),
            ..
        }) => raw.parse().ok(),
        _ => None,
    }
}

fn sort_by(order: &OrderBy, rows: &mut [Value]) -> Result<()> {
    let OrderByKind::Expressions(exprs) = &order.kind else {
        return Err(Error::usage("sql_order", "ORDER BY ALL is not supported"));
    };
    rows.sort_by(|a, b| {
        for item in exprs {
            let lv = eval_expr(&item.expr, a).unwrap_or(Value::Null);
            let rv = eval_expr(&item.expr, b).unwrap_or(Value::Null);
            let mut ord = values_cmp(&lv, &rv);
            if item.options.asc == Some(false) {
                ord = ord.reverse();
            }
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::execute_sql;
    use crate::value::Value;

    #[test]
    fn select_where_limit() {
        let rows = vec![
            Value::object([
                ("region".into(), Value::String("west".into())),
                ("revenue".into(), Value::Int(10)),
            ]),
            Value::object([
                ("region".into(), Value::String("east".into())),
                ("revenue".into(), Value::Int(5)),
            ]),
        ];
        let mut tables = HashMap::new();
        tables.insert("input".into(), rows);
        let out = execute_sql(
            "SELECT region, revenue FROM input WHERE revenue > 6 LIMIT 1",
            &tables,
        )
        .expect("sql");
        assert_eq!(out.len(), 1);
    }

    fn tables_ab() -> HashMap<String, Vec<Value>> {
        let mut tables = HashMap::new();
        tables.insert(
            "a".into(),
            vec![
                Value::object([
                    ("id".into(), Value::Int(1)),
                    ("name".into(), Value::String("a1".into())),
                ]),
                Value::object([
                    ("id".into(), Value::Int(2)),
                    ("name".into(), Value::String("a2".into())),
                ]),
            ],
        );
        tables.insert(
            "b".into(),
            vec![
                Value::object([
                    ("id".into(), Value::Int(1)),
                    ("x".into(), Value::String("b1".into())),
                ]),
                Value::object([
                    ("id".into(), Value::Int(99)),
                    ("x".into(), Value::String("b99".into())),
                ]),
            ],
        );
        tables
    }

    #[test]
    fn inner_join_on_shared_id_is_equi_not_cartesian() {
        let out = execute_sql(
            "SELECT name, x FROM a INNER JOIN b ON a.id = b.id",
            &tables_ab(),
        )
        .expect("sql");
        assert_eq!(out.len(), 1);
        let row = out[0].as_object().expect("object");
        assert_eq!(row.get("name"), Some(&Value::String("a1".into())));
        assert_eq!(row.get("x"), Some(&Value::String("b1".into())));
    }

    #[test]
    fn left_join_keeps_unmatched_left_row() {
        let out = execute_sql(
            "SELECT name, x FROM a LEFT JOIN b ON a.id = b.id",
            &tables_ab(),
        )
        .expect("sql");
        assert_eq!(out.len(), 2);
        let names: Vec<_> = out
            .iter()
            .filter_map(Value::as_object)
            .filter_map(|row| row.get("name"))
            .collect();
        assert!(names.contains(&&Value::String("a1".into())));
        assert!(names.contains(&&Value::String("a2".into())));
    }

    #[test]
    fn inner_join_on_distinct_key_names() {
        let mut tables = HashMap::new();
        tables.insert(
            "a".into(),
            vec![Value::object([
                ("id".into(), Value::Int(1)),
                ("name".into(), Value::String("a1".into())),
            ])],
        );
        tables.insert(
            "b".into(),
            vec![Value::object([
                ("aid".into(), Value::Int(1)),
                ("x".into(), Value::String("b1".into())),
            ])],
        );
        let out = execute_sql(
            "SELECT name, x FROM a INNER JOIN b ON a.id = b.aid",
            &tables,
        )
        .expect("sql");
        assert_eq!(out.len(), 1);
        let row = out[0].as_object().expect("object");
        assert_eq!(row.get("name"), Some(&Value::String("a1".into())));
        assert_eq!(row.get("x"), Some(&Value::String("b1".into())));
    }
}
