//! Shrinks generated tool schemas: agents read them on every session, so
//! every byte counts. The meaning is unchanged: optional fields stay
//! optional (they are not `required`), only redundant detail goes.

use serde_json::{Map, Value};

/// Compacts a JSON schema in place.
pub(crate) fn compact(value: &mut Value) {
    match value {
        Value::Object(map) => compact_object(map),
        Value::Array(items) => items.iter_mut().for_each(compact),
        _ => {}
    }
}

fn compact_object(map: &mut Map<String, Value>) {
    for key in [
        "format",
        "writeOnly",
        "readOnly",
        "$schema",
        "title",
        "minimum",
        "maximum",
    ] {
        map.remove(key);
    }
    // A default that is what absence already means — `false`, empty, null —
    // says nothing. (A field named `default` is an object, never one of these.)
    if map.get("default").is_some_and(|d| {
        matches!(d, Value::Bool(false) | Value::Null)
            || d.as_array().is_some_and(Vec::is_empty)
            || d.as_object().is_some_and(Map::is_empty)
            || d.as_str().is_some_and(str::is_empty)
    }) {
        map.remove("default");
    }
    // `["number", "null"]` → `"number"`: absence already means null.
    if let Some(Value::Array(types)) = map.get("type") {
        let kept: Vec<Value> = types.iter().filter(|t| *t != "null").cloned().collect();
        if kept.len() == 1 {
            map.insert("type".into(), kept[0].clone());
        }
    }
    // `anyOf: [X, {type: null}]` → X merged in place.
    if let Some(Value::Array(options)) = map.get("anyOf") {
        let kept: Vec<Value> = options
            .iter()
            .filter(|o| o.get("type").is_none_or(|t| t != "null"))
            .cloned()
            .collect();
        if kept.len() == 1
            && let Value::Object(inner) = &kept[0]
        {
            map.remove("anyOf");
            for (k, v) in inner {
                map.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }
    if let Some(Value::String(text)) = map.get_mut("description") {
        let short = text.split_whitespace().collect::<Vec<_>>().join(" ");
        short.trim_end_matches('.').clone_into(text);
        if text.is_empty() {
            map.remove("description");
        }
    }
    for (key, value) in map.iter_mut() {
        // Under these the keys are names — of arguments, of definitions —
        // not keywords: each value is a schema, the map itself is not.
        match (key.as_str(), value) {
            ("properties" | "$defs" | "definitions", Value::Object(named)) => {
                named.values_mut().for_each(compact);
            }
            (_, value) => compact(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_nulls_formats_and_trailing_dots() {
        let mut schema = serde_json::json!({
            "properties": {
                "t": {"description": "Thickness\n cm.", "format": "double", "type": ["number", "null"]},
                "a": {"anyOf": [{"$ref": "#/$defs/Point2"}, {"type": "null"}], "description": "Start."}
            }
        });
        compact(&mut schema);
        assert_eq!(
            schema,
            serde_json::json!({
                "properties": {
                    "t": {"description": "Thickness cm", "type": "number"},
                    "a": {"$ref": "#/$defs/Point2", "description": "Start"}
                }
            })
        );
    }

    /// An argument named like a keyword (`format`, `title`, `default`) is
    /// an argument: only a schema's own keywords go.
    #[test]
    fn an_argument_named_like_a_keyword_stays() {
        let mut schema = serde_json::json!({
            "properties": {
                "format": {"type": ["string", "null"], "title": "F", "description": "csv."},
                "title": {"type": "string", "format": "x"},
                "default": {"type": "boolean"},
            },
            "$defs": {"title": {"type": "string", "title": "T"}},
        });
        compact(&mut schema);
        assert_eq!(
            schema,
            serde_json::json!({
                "properties": {
                    "format": {"type": "string", "description": "csv"},
                    "title": {"type": "string"},
                    "default": {"type": "boolean"},
                },
                "$defs": {"title": {"type": "string"}},
            })
        );
    }
}
