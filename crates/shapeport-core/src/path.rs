//! ShapePort FieldPath grammar (RFC 0001 §8).

use std::fmt::{Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Error, Result};

/// One path segment: a field name or a 0-based list index.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum PathSegment {
    Field(String),
    Index(usize),
}

/// Record-relative field path: `field`, `field.sub`, `field[N]`.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct FieldPath {
    segments: Vec<PathSegment>,
}

impl FieldPath {
    /// Parse a FieldPath. JSONPath (`$`, `..`, filters) is rejected.
    pub fn parse(input: &str) -> Result<Self> {
        reject_jsonpath(input)?;
        let segments = parse_segments(input)?;
        if segments.is_empty() {
            return Err(Error::plan("empty_path", "FieldPath must not be empty"));
        }
        Ok(Self { segments })
    }

    #[must_use]
    pub fn segments(&self) -> &[PathSegment] {
        &self.segments
    }

    #[must_use]
    pub fn is_simple_field(&self) -> bool {
        matches!(self.segments.as_slice(), [PathSegment::Field(_)])
    }

    #[must_use]
    pub fn simple_name(&self) -> Option<&str> {
        match self.segments.as_slice() {
            [PathSegment::Field(name)] => Some(name.as_str()),
            _ => None,
        }
    }
}

impl Display for FieldPath {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&render_path(&self.segments))
    }
}

impl FromStr for FieldPath {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        Self::parse(input)
    }
}

impl Serialize for FieldPath {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for FieldPath {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

fn reject_jsonpath(input: &str) -> Result<()> {
    let invalid = input.starts_with('$')
        || input.contains("..")
        || input.contains('?')
        || input.contains('@')
        || input.contains('*')
        || input.contains('\'')
        || input.contains('"');
    if invalid {
        return Err(Error::plan(
            "jsonpath_forbidden",
            format!("JSONPath is not allowed in FieldPath: {input}"),
        ));
    }
    Ok(())
}

fn parse_segments(input: &str) -> Result<Vec<PathSegment>> {
    let mut segments = Vec::new();
    let mut rest = input;
    while !rest.is_empty() {
        rest = parse_one(rest, &mut segments)?;
    }
    Ok(segments)
}

fn parse_one<'a>(rest: &'a str, segments: &mut Vec<PathSegment>) -> Result<&'a str> {
    if rest.starts_with('[') {
        return parse_index(rest, segments);
    }
    parse_ident(rest, segments)
}

fn parse_ident<'a>(rest: &'a str, segments: &mut Vec<PathSegment>) -> Result<&'a str> {
    let ident_len = ident_len(rest)?;
    segments.push(PathSegment::Field(rest[..ident_len].to_string()));
    let after = &rest[ident_len..];
    skip_dot_or_keep(after)
}

fn ident_len(rest: &str) -> Result<usize> {
    let mut chars = rest.chars();
    let first = chars
        .next()
        .ok_or_else(|| Error::plan("invalid_path", "expected identifier in FieldPath"))?;
    if !is_ident_start(first) {
        return Err(Error::plan(
            "invalid_path",
            format!("invalid FieldPath identifier start: {rest}"),
        ));
    }
    let mut len = first.len_utf8();
    for ch in chars {
        if is_ident_continue(ch) {
            len += ch.len_utf8();
        } else {
            break;
        }
    }
    Ok(len)
}

fn parse_index<'a>(rest: &'a str, segments: &mut Vec<PathSegment>) -> Result<&'a str> {
    let close = rest
        .find(']')
        .ok_or_else(|| Error::plan("invalid_path", "unterminated index in FieldPath"))?;
    let inner = &rest[1..close];
    if inner.is_empty() || (inner.len() > 1 && inner.starts_with('0')) {
        return Err(Error::plan(
            "invalid_path",
            format!("invalid FieldPath index: [{inner}]"),
        ));
    }
    if !inner.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(Error::plan(
            "invalid_path",
            format!("invalid FieldPath index: [{inner}]"),
        ));
    }
    let index = inner.parse::<usize>().map_err(|_| {
        Error::plan(
            "invalid_path",
            format!("FieldPath index overflow: [{inner}]"),
        )
    })?;
    segments.push(PathSegment::Index(index));
    skip_dot_or_keep(&rest[close + 1..])
}

fn skip_dot_or_keep(after: &str) -> Result<&str> {
    if after.is_empty() || after.starts_with('[') {
        return Ok(after);
    }
    if let Some(stripped) = after.strip_prefix('.') {
        if stripped.is_empty() {
            return Err(Error::plan("invalid_path", "trailing '.' in FieldPath"));
        }
        return Ok(stripped);
    }
    Err(Error::plan(
        "invalid_path",
        format!("unexpected FieldPath remainder: {after}"),
    ))
}

const fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

const fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn render_path(segments: &[PathSegment]) -> String {
    let mut out = String::new();
    for segment in segments {
        match segment {
            PathSegment::Field(name) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(name);
            }
            PathSegment::Index(index) => {
                out.push('[');
                out.push_str(&index.to_string());
                out.push(']');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{FieldPath, PathSegment};

    #[test]
    fn parses_nested_and_index() {
        let path = FieldPath::parse("items[0].sku").expect("path");
        assert_eq!(
            path.segments(),
            &[
                PathSegment::Field("items".into()),
                PathSegment::Index(0),
                PathSegment::Field("sku".into())
            ]
        );
        assert_eq!(path.to_string(), "items[0].sku");
    }

    #[test]
    fn rejects_jsonpath() {
        assert!(FieldPath::parse("$.period").is_err());
        assert!(FieldPath::parse("$[*]").is_err());
        assert!(FieldPath::parse("..revenue").is_err());
        assert!(FieldPath::parse("items[?(@.x>0)]").is_err());
    }

    #[test]
    fn rejects_leading_zero_index() {
        assert!(FieldPath::parse("items[01]").is_err());
    }
}
