//! Document value tree used by the document VM.

use base64::Engine as _;
use indexmap::IndexMap;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{Display, Formatter};

/// Decimal stored as coefficient + scale; JSON encoding is a decimal string.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct DecimalValue {
    pub coefficient: String,
    pub scale: i8,
}

impl DecimalValue {
    #[must_use]
    pub fn parse_str(raw: &str) -> Option<Self> {
        parse_decimal(raw)
    }

    #[must_use]
    pub fn to_canonical_string(&self) -> String {
        render_decimal(self)
    }
}

impl Display for DecimalValue {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.to_canonical_string())
    }
}

/// `ShapePort` document value. Object insertion order is significant.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Decimal(DecimalValue),
    String(String),
    Binary(Vec<u8>),
    Date(i32),
    TimestampMicros(i64),
    Array(Vec<Value>),
    Object(IndexMap<String, Value>),
}

impl Value {
    #[must_use]
    pub fn object(pairs: impl IntoIterator<Item = (String, Self)>) -> Self {
        Self::Object(pairs.into_iter().collect())
    }

    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    #[must_use]
    pub fn as_object(&self) -> Option<&IndexMap<String, Self>> {
        match self {
            Self::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut IndexMap<String, Self>> {
        match self {
            Self::Object(map) => Some(map),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(flag) => Some(*flag),
            _ => None,
        }
    }

    /// Truthiness for filter predicates: null/false are false; all else true.
    #[must_use]
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Self::Null | Self::Bool(false))
    }
}

impl Serialize for Value {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Null => serializer.serialize_none(),
            Self::Bool(flag) => serializer.serialize_bool(*flag),
            Self::Int(value) => serializer.serialize_i64(*value),
            Self::UInt(value) => serializer.serialize_u64(*value),
            Self::Float(value) => serializer.serialize_f64(*value),
            Self::Decimal(value) => serializer.serialize_str(&value.to_canonical_string()),
            Self::String(text) => serializer.serialize_str(text),
            Self::Binary(bytes) => {
                serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
            }
            Self::Date(days) => serializer.serialize_i32(*days),
            Self::TimestampMicros(micros) => serializer.serialize_i64(*micros),
            Self::Array(items) => items.serialize(serializer),
            Self::Object(map) => map.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Int(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Value, E> {
        match i64::try_from(value) {
            Ok(signed) => Ok(Value::Int(signed)),
            Err(_) => Ok(Value::UInt(value)),
        }
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        Ok(Value::Float(value))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_string()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = access.next_element()? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let mut map = IndexMap::new();
        while let Some((key, val)) = access.next_entry()? {
            map.insert(key, val);
        }
        Ok(Value::Object(map))
    }
}

fn parse_decimal(raw: &str) -> Option<DecimalValue> {
    if raw.is_empty() {
        return None;
    }
    let negative = raw.starts_with('-');
    let body = raw.strip_prefix('-').unwrap_or(raw);
    if body.is_empty() || !body.chars().all(|ch| ch.is_ascii_digit() || ch == '.') {
        return None;
    }
    if body.matches('.').count() > 1 {
        return None;
    }
    let (int_part, frac) = split_decimal_parts(body)?;
    if int_part.is_empty() {
        return None;
    }
    let scale = i8::try_from(frac.len()).ok()?;
    let mut coefficient = String::new();
    if negative {
        coefficient.push('-');
    }
    coefficient.push_str(int_part.trim_start_matches('0'));
    if coefficient == "-" || coefficient.is_empty() {
        coefficient = if negative { "-0".into() } else { "0".into() };
    }
    coefficient.push_str(frac);
    if coefficient == "-" || coefficient == "-0" && frac.is_empty() {
        coefficient = "0".into();
    }
    Some(DecimalValue { coefficient, scale })
}

fn split_decimal_parts(body: &str) -> Option<(&str, &str)> {
    match body.split_once('.') {
        Some((int_part, frac)) => {
            if !frac.chars().all(|ch| ch.is_ascii_digit()) {
                return None;
            }
            Some((if int_part.is_empty() { "0" } else { int_part }, frac))
        }
        None => Some((body, "")),
    }
}

fn render_decimal(value: &DecimalValue) -> String {
    let negative = value.coefficient.starts_with('-');
    let digits = value.coefficient.trim_start_matches('-');
    let scale = usize::from(u8::try_from(value.scale.max(0)).unwrap_or(0));
    if scale == 0 {
        return if negative {
            format!("-{digits}")
        } else {
            digits.to_string()
        };
    }
    let padded = if digits.len() <= scale {
        format!("{:0>width$}", digits, width = scale + 1)
    } else {
        digits.to_string()
    };
    let split_at = padded.len() - scale;
    let rendered = format!("{}.{}", &padded[..split_at], &padded[split_at..]);
    if negative {
        format!("-{rendered}")
    } else {
        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::{DecimalValue, Value};

    #[test]
    fn decimal_round_trip_string() {
        let parsed = DecimalValue::parse_str("12340.20").expect("decimal");
        assert_eq!(parsed.scale, 2);
        assert_eq!(parsed.to_canonical_string(), "12340.20");
    }

    #[test]
    fn json_preserves_object_order() {
        let value = Value::object([
            ("month".into(), Value::String("2026-01".into())),
            ("product".into(), Value::String("Compute".into())),
        ]);
        let json = serde_json::to_string(&value).expect("json");
        assert_eq!(json, r#"{"month":"2026-01","product":"Compute"}"#);
    }
}
