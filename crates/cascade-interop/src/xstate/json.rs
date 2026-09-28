//! A JSON value that keeps object keys in document order.
//!
//! State order matters (it is the display order, and Cascade's default
//! initial state is the first one), so XState configs are read into this
//! instead of `serde_json::Value`, whose map is sorted unless a
//! workspace-wide feature is enabled.

use std::fmt;

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    /// The value of `key` in an object (the last one, as `JSON.parse` does).
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(entries) => entries.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Object(entries) => Some(entries),
            _ => None,
        }
    }

    /// A short description of the value's type, for error messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Json::Null => "null",
            Json::Bool(_) => "a boolean",
            Json::Number(_) => "a number",
            Json::String(_) => "a string",
            Json::Array(_) => "an array",
            Json::Object(_) => "an object",
        }
    }

    /// Compact JSON text, for guards with parameters.
    pub fn to_compact(&self) -> String {
        match self {
            Json::Null => "null".to_owned(),
            Json::Bool(b) => b.to_string(),
            Json::Number(n) => n.to_string(),
            Json::String(s) => serde_json::Value::String(s.clone()).to_string(),
            Json::Array(items) => format!("[{}]", items.iter().map(Json::to_compact).collect::<Vec<_>>().join(",")),
            Json::Object(entries) => format!(
                "{{{}}}",
                entries
                    .iter()
                    .map(|(k, v)| format!("{}:{}", serde_json::Value::String(k.clone()), v.to_compact()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Json;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_unit<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_none<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_bool<E>(self, v: bool) -> Result<Json, E> {
        Ok(Json::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Json, E> {
        // JSON numbers are doubles; precision loss past 2^53 matches JS.
        #[allow(clippy::cast_precision_loss)]
        Ok(Json::Number(v as f64))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Json, E> {
        #[allow(clippy::cast_precision_loss)]
        Ok(Json::Number(v as f64))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Json, E> {
        Ok(Json::Number(v))
    }

    fn visit_str<E>(self, v: &str) -> Result<Json, E> {
        Ok(Json::String(v.to_owned()))
    }

    fn visit_string<E>(self, v: String) -> Result<Json, E> {
        Ok(Json::String(v))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Json::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        let mut entries = Vec::new();
        while let Some((key, value)) = map.next_entry::<String, Json>()? {
            entries.push((key, value));
        }
        Ok(Json::Object(entries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Json {
        match serde_json::from_str(text) {
            Ok(json) => json,
            Err(err) => panic!("{err}"),
        }
    }

    #[test]
    fn keeps_object_key_order() {
        let json = parse(r#"{ "zeta": 1, "alpha": [true, null, "x"], "mid": { "b": 2, "a": 3 } }"#);
        let keys: Vec<&str> = json.as_object().map(|o| o.iter().map(|(k, _)| k.as_str()).collect()).unwrap_or_default();
        assert_eq!(keys, ["zeta", "alpha", "mid"]);
        assert_eq!(json.get("alpha"), Some(&Json::Array(vec![Json::Bool(true), Json::Null, Json::String("x".into())])));
        assert_eq!(json.to_compact(), r#"{"zeta":1,"alpha":[true,null,"x"],"mid":{"b":2,"a":3}}"#);
    }

    #[test]
    fn duplicate_keys_resolve_to_the_last() {
        let json = parse(r#"{ "a": 1, "a": 2 }"#);
        assert_eq!(json.get("a"), Some(&Json::Number(2.0)));
    }
}
