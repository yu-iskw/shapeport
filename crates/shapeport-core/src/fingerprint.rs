//! Schema fingerprints: `sha256:<hex>` over canonical JSON excluding metadata.

use serde_json::{Map, Value as Json};
use sha2::{Digest, Sha256};

use crate::schema::{Field, Schema, Type};

/// SHA-256 fingerprint of a schema AST with metadata stripped.
#[must_use]
pub fn schema_fingerprint(schema: &Schema) -> String {
    let stripped = type_to_json(&schema.root);
    let encoded = canonical_json(&stripped);
    let digest = Sha256::digest(encoded.as_bytes());
    format!("sha256:{}", hex::encode(digest))
}

fn type_to_json(ty: &Type) -> Json {
    match ty {
        Type::Null => tagged("null", Map::new()),
        Type::Bool => tagged("bool", Map::new()),
        Type::Int { bits, signed } => {
            let mut map = Map::new();
            map.insert("bits".into(), Json::from(*bits));
            map.insert("signed".into(), Json::from(*signed));
            tagged("int", map)
        }
        Type::Float { bits } => {
            let mut map = Map::new();
            map.insert("bits".into(), Json::from(*bits));
            tagged("float", map)
        }
        Type::Decimal { precision, scale } => {
            let mut map = Map::new();
            map.insert("precision".into(), Json::from(*precision));
            map.insert("scale".into(), Json::from(*scale));
            tagged("decimal", map)
        }
        Type::String => tagged("string", Map::new()),
        Type::Binary => tagged("binary", Map::new()),
        Type::Date => tagged("date", Map::new()),
        Type::Time { unit } => unit_tag("time", *unit),
        Type::Timestamp { unit, timezone } => {
            let mut map = Map::new();
            map.insert(
                "unit".into(),
                Json::String(format!("{unit:?}").to_ascii_lowercase()),
            );
            if let Some(tz) = timezone {
                map.insert("timezone".into(), Json::String(tz.clone()));
            }
            tagged("timestamp", map)
        }
        Type::Duration { unit } => unit_tag("duration", *unit),
        Type::Record { fields } => {
            let mut map = Map::new();
            map.insert(
                "fields".into(),
                Json::Array(fields.iter().map(field_to_json).collect()),
            );
            tagged("record", map)
        }
        Type::List {
            element,
            element_nullable,
        } => {
            let mut map = Map::new();
            map.insert("element".into(), type_to_json(element));
            map.insert("elementNullable".into(), Json::from(*element_nullable));
            tagged("list", map)
        }
        Type::Map {
            key,
            value,
            value_nullable,
        } => {
            let mut map = Map::new();
            map.insert("key".into(), type_to_json(key));
            map.insert("value".into(), type_to_json(value));
            map.insert("valueNullable".into(), Json::from(*value_nullable));
            tagged("map", map)
        }
        Type::Union { variants } => {
            let mut map = Map::new();
            map.insert(
                "variants".into(),
                Json::Array(variants.iter().map(type_to_json).collect()),
            );
            tagged("union", map)
        }
        Type::Unknown => tagged("unknown", Map::new()),
        Type::Any => tagged("any", Map::new()),
    }
}

fn unit_tag(kind: &str, unit: crate::schema::TimeUnit) -> Json {
    let mut map = Map::new();
    map.insert(
        "unit".into(),
        Json::String(format!("{unit:?}").to_ascii_lowercase()),
    );
    tagged(kind, map)
}

fn field_to_json(field: &Field) -> Json {
    let mut map = Map::new();
    map.insert("name".into(), Json::String(field.name.clone()));
    map.insert("nullable".into(), Json::from(field.nullable));
    map.insert("type".into(), type_to_json(&field.ty));
    if !field.aliases.is_empty() {
        map.insert(
            "aliases".into(),
            Json::Array(field.aliases.iter().cloned().map(Json::String).collect()),
        );
    }
    if let Some(semantic) = &field.semantic {
        map.insert("semantic".into(), Json::String(semantic.clone()));
    }
    Json::Object(map)
}

fn tagged(kind: &str, mut fields: Map<String, Json>) -> Json {
    fields.insert("kind".into(), Json::String(kind.to_string()));
    Json::Object(fields)
}

fn canonical_json(value: &Json) -> String {
    match value {
        Json::Null => "null".into(),
        Json::Bool(flag) => {
            if *flag {
                "true".into()
            } else {
                "false".into()
            }
        }
        Json::Number(number) => number.to_string(),
        Json::String(text) => serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into()),
        Json::Array(items) => {
            let inner: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        Json::Object(map) => encode_object(map),
    }
}

fn encode_object(map: &Map<String, Json>) -> String {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    let mut parts = Vec::new();
    for key in keys {
        if let Some(val) = map.get(key) {
            let encoded_key = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".into());
            parts.push(format!("{encoded_key}:{}", canonical_json(val)));
        }
    }
    format!("{{{}}}", parts.join(","))
}

#[cfg(test)]
mod tests {
    use crate::schema::{Field, Schema, Type};

    use super::schema_fingerprint;

    #[test]
    fn metadata_does_not_change_fingerprint() {
        let mut left = Schema::record(vec![Field::new("id", Type::String, false)]);
        let right = left.clone();
        left.metadata
            .insert("note".into(), serde_json::json!("ignored"));
        assert_eq!(schema_fingerprint(&left), schema_fingerprint(&right));
        assert!(schema_fingerprint(&left).starts_with("sha256:"));
    }

    #[test]
    fn field_order_changes_fingerprint() {
        let a = Schema::record(vec![
            Field::new("a", Type::String, false),
            Field::new("b", Type::String, false),
        ]);
        let b = Schema::record(vec![
            Field::new("b", Type::String, false),
            Field::new("a", Type::String, false),
        ]);
        assert_ne!(schema_fingerprint(&a), schema_fingerprint(&b));
    }
}
