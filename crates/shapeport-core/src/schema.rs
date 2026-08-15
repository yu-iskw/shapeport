//! Core schema model (RFC 0001 §7).

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Timestamp / time / duration unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimeUnit {
    Second,
    Millisecond,
    Microsecond,
    Nanosecond,
}

/// Record field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: Type,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

impl Field {
    #[must_use]
    pub fn new(name: impl Into<String>, ty: Type, nullable: bool) -> Self {
        Self {
            name: name.into(),
            ty,
            nullable,
            aliases: Vec::new(),
            semantic: None,
            metadata: serde_json::Map::new(),
        }
    }
}

/// `ShapePort` type AST. Field order in records is significant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Type {
    Null,
    Bool,
    Int {
        bits: u8,
        signed: bool,
    },
    Float {
        bits: u8,
    },
    Decimal {
        precision: u8,
        scale: i8,
    },
    String,
    Binary,
    Date,
    Time {
        unit: TimeUnit,
    },
    Timestamp {
        unit: TimeUnit,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timezone: Option<String>,
    },
    Duration {
        unit: TimeUnit,
    },
    Record {
        fields: Vec<Field>,
    },
    List {
        element: Box<Type>,
        element_nullable: bool,
    },
    Map {
        key: Box<Type>,
        value: Box<Type>,
        value_nullable: bool,
    },
    Union {
        variants: Vec<Type>,
    },
    Unknown,
    Any,
}

impl Type {
    #[must_use]
    pub fn record(fields: Vec<Field>) -> Self {
        Self::Record { fields }
    }

    #[must_use]
    pub fn list(element: Type, element_nullable: bool) -> Self {
        Self::List {
            element: Box::new(element),
            element_nullable,
        }
    }

    #[must_use]
    pub const fn is_nested(&self) -> bool {
        matches!(
            self,
            Self::Record { .. } | Self::List { .. } | Self::Map { .. } | Self::Union { .. }
        )
    }

    #[must_use]
    pub fn as_record_fields(&self) -> Option<&[Field]> {
        match self {
            Self::Record { fields } => Some(fields),
            _ => None,
        }
    }

    /// Named scalar used in plan `cast.target`.
    pub fn from_target_name(name: &str) -> Result<Self> {
        parse_target_name(name)
    }

    #[must_use]
    pub fn target_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool => "bool",
            Self::Int {
                signed: true,
                bits: 64,
            } => "int64",
            Self::Int {
                signed: true,
                bits: 32,
            } => "int32",
            Self::Int {
                signed: false,
                bits: 64,
            } => "uint64",
            Self::Float { bits: 64 } => "float64",
            Self::Float { bits: 32 } => "float32",
            Self::Decimal { .. } => "decimal",
            Self::String => "string",
            Self::Binary => "binary",
            Self::Date => "date",
            Self::Timestamp { .. } => "timestamp",
            Self::Unknown => "unknown",
            Self::Any => "any",
            _ => "complex",
        }
    }
}

fn parse_target_name(name: &str) -> Result<Type> {
    match name {
        "null" => Ok(Type::Null),
        "bool" | "boolean" => Ok(Type::Bool),
        "int" | "int64" => Ok(Type::Int {
            bits: 64,
            signed: true,
        }),
        "int32" => Ok(Type::Int {
            bits: 32,
            signed: true,
        }),
        "uint64" => Ok(Type::Int {
            bits: 64,
            signed: false,
        }),
        "float" | "float64" | "number" => Ok(Type::Float { bits: 64 }),
        "float32" => Ok(Type::Float { bits: 32 }),
        "decimal" => Ok(Type::Decimal {
            precision: 38,
            scale: 10,
        }),
        "string" => Ok(Type::String),
        "binary" => Ok(Type::Binary),
        "date" => Ok(Type::Date),
        "timestamp" => Ok(Type::Timestamp {
            unit: TimeUnit::Microsecond,
            timezone: None,
        }),
        other => Err(Error::plan(
            "unknown_cast_target",
            format!("unknown cast target {other}"),
        )),
    }
}

/// A schema with a root type and optional metadata (excluded from fingerprints).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schema {
    pub root: Type,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

impl Schema {
    #[must_use]
    pub fn new(root: Type) -> Self {
        Self {
            root,
            metadata: serde_json::Map::new(),
        }
    }

    #[must_use]
    pub fn record(fields: Vec<Field>) -> Self {
        Self::new(Type::record(fields))
    }

    #[must_use]
    pub fn fields(&self) -> Option<&[Field]> {
        self.root.as_record_fields()
    }

    pub fn require_record_fields(&self) -> Result<&[Field]> {
        self.fields().ok_or_else(|| {
            Error::schema(
                "not_a_record",
                "schema root must be a record for this operation",
            )
        })
    }
}

/// Assignability used by the planner (not full JSON Schema validation).
#[must_use]
pub fn types_compatible(source: &Type, target: &Type) -> bool {
    if matches!(target, Type::Any) || matches!(source, Type::Unknown) {
        return true;
    }
    if matches!((source, target), (Type::List { .. }, Type::List { .. })) {
        return source == target;
    }
    if std::mem::discriminant(source) == std::mem::discriminant(target) {
        return true;
    }
    matches!(
        (source, target),
        (
            Type::Int { .. },
            Type::Float { .. } | Type::Decimal { .. } | Type::String
        ) | (Type::Decimal { .. }, Type::String | Type::Float { .. })
            | (Type::Float { .. } | Type::Bool, Type::String)
    )
}

#[cfg(test)]
mod tests {
    use super::{Field, Schema, Type, types_compatible};

    #[test]
    fn record_field_order_is_preserved() {
        let schema = Schema::record(vec![
            Field::new("month", Type::String, false),
            Field::new("revenue", Type::String, false),
        ]);
        let names: Vec<_> = schema
            .fields()
            .expect("record")
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(names, ["month", "revenue"]);
    }

    #[test]
    fn list_compatibility_requires_exact_element_type() {
        let source = Type::list(
            Type::record(vec![Field::new("first_name", Type::String, false)]),
            false,
        );
        let target = Type::list(
            Type::record(vec![Field::new("firstName", Type::String, false)]),
            false,
        );
        assert!(!types_compatible(&source, &target));
        assert!(types_compatible(&source, &source));
    }
}
