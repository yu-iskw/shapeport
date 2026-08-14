//! Deterministic function registry.

use crate::error::{Error, Result};
use crate::value::Value;

pub fn call(name: &str, args: &[Value]) -> Result<Value> {
    match name {
        "lower" => unary_string(name, args, str::to_lowercase),
        "upper" => unary_string(name, args, str::to_uppercase),
        "trim" => unary_string(name, args, |s| s.trim().to_string()),
        "concat" => Ok(concat_args(args)),
        "replace" => replace_args(args),
        "substring" => substring_args(args),
        "regex_extract" => regex_extract(args),
        "abs" => numeric_unary(name, args, abs_num),
        "round" => numeric_unary(name, args, round_num),
        "floor" => numeric_unary(name, args, floor_num),
        "ceil" => numeric_unary(name, args, ceil_num),
        "length" => length_of(args),
        "is_null" => Ok(Value::Bool(args.first().is_none_or(Value::is_null))),
        "null_if" => null_if(args),
        "coalesce" => Ok(coalesce(args)),
        "parse_date" | "parse_timestamp" | "format_timestamp" | "date_trunc" => {
            temporal(name, args)
        }
        other => Err(Error::plan(
            "unknown_function",
            format!("unknown function {other}"),
        )),
    }
}

fn unary_string(name: &str, args: &[Value], map: fn(&str) -> String) -> Result<Value> {
    let text = one_string(name, args)?;
    Ok(Value::String(map(&text)))
}

fn one_string(name: &str, args: &[Value]) -> Result<String> {
    match args {
        [Value::String(text)] => Ok(text.clone()),
        [Value::Null] => Err(Error::transform(
            "null_arg",
            format!("{name} does not accept null"),
        )),
        _ => Err(Error::transform(
            "bad_args",
            format!("{name} expects one string argument"),
        )),
    }
}

fn concat_args(args: &[Value]) -> Value {
    let mut out = String::new();
    for arg in args {
        match arg {
            Value::Null => {}
            Value::String(text) => out.push_str(text),
            other => out.push_str(&value_as_text(other)),
        }
    }
    Value::String(out)
}

fn replace_args(args: &[Value]) -> Result<Value> {
    match args {
        [Value::String(src), Value::String(from), Value::String(to)] => {
            Ok(Value::String(src.replace(from, to)))
        }
        _ => Err(Error::transform(
            "bad_args",
            "replace expects (string, from, to)",
        )),
    }
}

fn substring_args(args: &[Value]) -> Result<Value> {
    let (src, start, len) = match args {
        [Value::String(src), start] => (src, int_arg(start)?, None),
        [Value::String(src), start, len] => (src, int_arg(start)?, Some(int_arg(len)?)),
        _ => {
            return Err(Error::transform(
                "bad_args",
                "substring expects (string, start[, length])",
            ));
        }
    };
    let start = usize::try_from(start.max(0)).unwrap_or(0);
    let sliced = match len {
        Some(length) => {
            let length = usize::try_from(length.max(0)).unwrap_or(0);
            src.chars().skip(start).take(length).collect()
        }
        None => src.chars().skip(start).collect(),
    };
    Ok(Value::String(sliced))
}

fn regex_extract(args: &[Value]) -> Result<Value> {
    let [Value::String(src), Value::String(pattern)] = args else {
        return Err(Error::transform(
            "bad_args",
            "regex_extract expects (string, pattern)",
        ));
    };
    if pattern.len() > 256 {
        return Err(Error::limit(
            "regex_size",
            "regex pattern exceeds 256 characters",
        ));
    }
    let re = regex::Regex::new(pattern)
        .map_err(|err| Error::transform("invalid_regex", err.to_string()))?;
    match re.find(src) {
        Some(found) => Ok(Value::String(found.as_str().to_string())),
        None => Ok(Value::Null),
    }
}

fn numeric_unary(name: &str, args: &[Value], map: fn(f64) -> f64) -> Result<Value> {
    let number = match args {
        [value] => as_f64(value)?,
        _ => {
            return Err(Error::transform(
                "bad_args",
                format!("{name} expects one numeric argument"),
            ));
        }
    };
    Ok(Value::Float(map(number)))
}

fn abs_num(value: f64) -> f64 {
    value.abs()
}

fn round_num(value: f64) -> f64 {
    value.round()
}

fn floor_num(value: f64) -> f64 {
    value.floor()
}

fn ceil_num(value: f64) -> f64 {
    value.ceil()
}

fn length_of(args: &[Value]) -> Result<Value> {
    let len = match args {
        [Value::String(text)] => i64::try_from(text.chars().count()).unwrap_or(i64::MAX),
        [Value::Array(items)] => i64::try_from(items.len()).unwrap_or(i64::MAX),
        [Value::Object(map)] => i64::try_from(map.len()).unwrap_or(i64::MAX),
        [Value::Null] => 0,
        _ => {
            return Err(Error::transform(
                "bad_args",
                "length expects string, array, or object",
            ));
        }
    };
    Ok(Value::Int(len))
}

fn null_if(args: &[Value]) -> Result<Value> {
    match args {
        [left, right] if left == right => Ok(Value::Null),
        [left, _] => Ok(left.clone()),
        _ => Err(Error::transform(
            "bad_args",
            "null_if expects two arguments",
        )),
    }
}

fn coalesce(args: &[Value]) -> Value {
    args.iter()
        .find(|value| !value.is_null())
        .cloned()
        .unwrap_or(Value::Null)
}

fn temporal(name: &str, args: &[Value]) -> Result<Value> {
    match (name, args) {
        ("parse_timestamp" | "parse_date", [Value::String(text)])
        | ("format_timestamp" | "date_trunc", [Value::String(text), ..]) => {
            Ok(Value::String(text.clone()))
        }
        _ => Err(Error::transform(
            "bad_args",
            format!("{name} expects a timestamp string"),
        )),
    }
}

fn int_arg(value: &Value) -> Result<i64> {
    match value {
        Value::Int(v) => Ok(*v),
        Value::UInt(v) => i64::try_from(*v)
            .map_err(|_| Error::transform("overflow", "integer argument does not fit i64")),
        Value::Float(v) => Ok(*v as i64),
        _ => Err(Error::transform("bad_args", "expected integer argument")),
    }
}

fn as_f64(value: &Value) -> Result<f64> {
    match value {
        Value::Int(v) => Ok(*v as f64),
        Value::UInt(v) => Ok(*v as f64),
        Value::Float(v) => Ok(*v),
        Value::String(text) => text
            .parse::<f64>()
            .map_err(|_| Error::transform("cast_failed", format!("not numeric: {text}"))),
        Value::Decimal(dec) => dec
            .to_canonical_string()
            .parse::<f64>()
            .map_err(|_| Error::transform("cast_failed", "decimal is not finite")),
        _ => Err(Error::transform("cast_failed", "value is not numeric")),
    }
}

fn value_as_text(value: &Value) -> String {
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

#[cfg(test)]
mod tests {
    use super::call;
    use crate::value::Value;

    #[test]
    fn lower_and_concat() {
        let lower = call("lower", &[Value::String("AbC".into())]).expect("lower");
        assert_eq!(lower, Value::String("abc".into()));
        let joined = call(
            "concat",
            &[Value::String("a".into()), Value::String("b".into())],
        )
        .expect("concat");
        assert_eq!(joined, Value::String("ab".into()));
    }
}
