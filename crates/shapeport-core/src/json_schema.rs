//! JSON Schema 2020-12 practical subset → Core Schema, plus validation.

use serde_json::{Map, Value as Json};

use crate::error::{Error, Result};
use crate::schema::{Field, Schema, Type};
use crate::value::Value;

pub fn schema_from_json_schema(doc: &Json) -> Result<Schema> {
    Ok(Schema::new(type_from_schema(doc, doc)?))
}

pub fn schema_to_json_schema(schema: &Schema) -> Json {
    type_to_json_schema(&schema.root)
}

fn type_from_schema(node: &Json, root: &Json) -> Result<Type> {
    let node = resolve_local_ref(node, root)?;
    if let Some(union) = nullable_union(node, root)? {
        return Ok(union);
    }
    if let Some(one) = node.get("oneOf").or_else(|| node.get("anyOf")) {
        return union_from_list(one, root);
    }
    if let Some(all) = node.get("allOf").and_then(Json::as_array) {
        return merge_all_of(all, root);
    }
    scalar_or_compound(node, root)
}

fn scalar_or_compound(node: &Json, root: &Json) -> Result<Type> {
    match node.get("type").and_then(Json::as_str) {
        Some("object") => record_from_object(node, root),
        Some("array") => list_from_array(node, root),
        Some("string") => Ok(Type::String),
        Some("integer") => Ok(Type::Int {
            bits: 64,
            signed: true,
        }),
        Some("number") => Ok(Type::Float { bits: 64 }),
        Some("boolean") => Ok(Type::Bool),
        Some("null") => Ok(Type::Null),
        Some(other) => Err(Error::schema(
            "unsupported_json_schema_type",
            format!("unsupported JSON Schema type {other}"),
        )),
        None => {
            if node.get("properties").is_some() {
                record_from_object(node, root)
            } else {
                Ok(Type::Any)
            }
        }
    }
}

fn record_from_object(node: &Json, root: &Json) -> Result<Type> {
    let required = required_names(node);
    let mut fields = Vec::new();
    if let Some(props) = node.get("properties").and_then(Json::as_object) {
        for (name, spec) in props {
            let ty = type_from_schema(spec, root)?;
            let nullable = !required.contains(name);
            fields.push(Field::new(name, ty, nullable));
        }
    }
    Ok(Type::record(fields))
}

fn list_from_array(node: &Json, root: &Json) -> Result<Type> {
    let items = node.get("items").unwrap_or(&Json::Null);
    let element = if items.is_null() {
        Type::Any
    } else {
        type_from_schema(items, root)?
    };
    Ok(Type::list(element, true))
}

fn union_from_list(list: &Json, root: &Json) -> Result<Type> {
    let items = list
        .as_array()
        .ok_or_else(|| Error::schema("invalid_union", "oneOf/anyOf must be an array"))?;
    let mut variants = Vec::new();
    for item in items {
        variants.push(type_from_schema(item, root)?);
    }
    Ok(Type::Union { variants })
}

fn merge_all_of(all: &[Json], root: &Json) -> Result<Type> {
    let mut fields = Vec::new();
    for item in all {
        match type_from_schema(item, root)? {
            Type::Record { fields: more } => fields.extend(more),
            other if fields.is_empty() => return Ok(other),
            _ => {}
        }
    }
    Ok(Type::record(fields))
}

fn nullable_union(node: &Json, root: &Json) -> Result<Option<Type>> {
    let Some(types) = node.get("type").and_then(Json::as_array) else {
        return Ok(None);
    };
    let mut variants = Vec::new();
    for item in types {
        let mut clone = node.clone();
        if let Some(obj) = clone.as_object_mut() {
            obj.insert("type".into(), item.clone());
        }
        variants.push(type_from_schema(&clone, root)?);
    }
    Ok(Some(Type::Union { variants }))
}

fn required_names(node: &Json) -> Vec<String> {
    node.get("required")
        .and_then(Json::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Json::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn resolve_local_ref<'a>(node: &'a Json, root: &'a Json) -> Result<&'a Json> {
    let Some(reference) = node.get("$ref").and_then(Json::as_str) else {
        return Ok(node);
    };
    if let Some(name) = reference.strip_prefix("#/$defs/") {
        return root
            .pointer(&format!("/$defs/{name}"))
            .or_else(|| root.pointer(&format!("/definitions/{name}")))
            .ok_or_else(|| {
                Error::schema("unresolved_ref", format!("unresolved $ref {reference}"))
            });
    }
    if reference.starts_with("#/") {
        return root.pointer(&reference[1..]).ok_or_else(|| {
            Error::schema("unresolved_ref", format!("unresolved $ref {reference}"))
        });
    }
    Err(Error::schema(
        "remote_ref_denied",
        format!("external $ref is not fetched: {reference}"),
    ))
}

fn type_to_json_schema(ty: &Type) -> Json {
    match ty {
        Type::Null => json_type("null"),
        Type::Bool => json_type("boolean"),
        Type::Int { .. } => json_type("integer"),
        Type::Float { .. } => json_type("number"),
        Type::Decimal { .. } | Type::String | Type::Date | Type::Timestamp { .. } => {
            json_type("string")
        }
        Type::Binary => json_type("string"),
        Type::Record { fields } => record_schema(fields),
        Type::List { element, .. } => {
            let mut map = Map::new();
            map.insert("type".into(), Json::String("array".into()));
            map.insert("items".into(), type_to_json_schema(element));
            Json::Object(map)
        }
        Type::Union { variants } => {
            let mut map = Map::new();
            map.insert(
                "anyOf".into(),
                Json::Array(variants.iter().map(type_to_json_schema).collect()),
            );
            Json::Object(map)
        }
        _ => json_type("object"),
    }
}

fn record_schema(fields: &[Field]) -> Json {
    let mut props = Map::new();
    let mut required = Vec::new();
    for field in fields {
        props.insert(field.name.clone(), type_to_json_schema(&field.ty));
        if !field.nullable {
            required.push(Json::String(field.name.clone()));
        }
    }
    let mut map = Map::new();
    map.insert("type".into(), Json::String("object".into()));
    map.insert("properties".into(), Json::Object(props));
    if !required.is_empty() {
        map.insert("required".into(), Json::Array(required));
    }
    Json::Object(map)
}

fn json_type(name: &str) -> Json {
    let mut map = Map::new();
    map.insert("type".into(), Json::String(name.into()));
    Json::Object(map)
}

pub fn validate_value(schema: &Schema, value: &Value) -> Result<()> {
    validate_type(&schema.root, value, "$")
}

fn validate_type(ty: &Type, value: &Value, path: &str) -> Result<()> {
    if value.is_null() {
        return Ok(());
    }
    match (ty, value) {
        (Type::Any | Type::Unknown, _) => Ok(()),
        (Type::Bool, Value::Bool(_)) => Ok(()),
        (Type::Int { .. }, Value::Int(_) | Value::UInt(_)) => Ok(()),
        (Type::Float { .. }, Value::Float(_) | Value::Int(_) | Value::UInt(_)) => Ok(()),
        (Type::String | Type::Decimal { .. }, Value::String(_) | Value::Decimal(_)) => Ok(()),
        (Type::Record { fields }, Value::Object(map)) => validate_object(fields, map, path),
        (Type::List { element, .. }, Value::Array(items)) => validate_list(element, items, path),
        (Type::Union { variants }, _) => validate_union(variants, value, path),
        _ => Err(Error::target(
            "type_mismatch",
            format!("type mismatch at {path}"),
        )),
    }
}

fn validate_object(
    fields: &[Field],
    map: &indexmap::IndexMap<String, Value>,
    path: &str,
) -> Result<()> {
    for field in fields {
        match map.get(&field.name) {
            Some(value) => validate_type(&field.ty, value, &format!("{path}.{}", field.name))?,
            None if field.nullable => {}
            None => {
                return Err(Error::target(
                    "missing_field",
                    format!("missing required field {path}.{}", field.name),
                ));
            }
        }
    }
    Ok(())
}

fn validate_list(element: &Type, items: &[Value], path: &str) -> Result<()> {
    for (idx, item) in items.iter().enumerate() {
        validate_type(element, item, &format!("{path}[{idx}]"))?;
    }
    Ok(())
}

fn validate_union(variants: &[Type], value: &Value, path: &str) -> Result<()> {
    for variant in variants {
        if validate_type(variant, value, path).is_ok() {
            return Ok(());
        }
    }
    Err(Error::target(
        "type_mismatch",
        format!("value does not match union at {path}"),
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::schema_from_json_schema;

    #[test]
    fn object_properties_become_fields() {
        let doc = json!({
            "type": "object",
            "required": ["month"],
            "properties": {
                "month": {"type": "string"},
                "revenue": {"type": "string"}
            }
        });
        let schema = schema_from_json_schema(&doc).expect("schema");
        let fields = schema.fields().expect("record");
        assert_eq!(fields[0].name, "month");
        assert!(!fields[0].nullable);
        assert!(fields[1].nullable);
    }

    #[test]
    fn remote_ref_is_denied() {
        let doc = json!({"$ref": "https://example.com/schema.json"});
        assert!(schema_from_json_schema(&doc).is_err());
    }
}
