//! JSON-to-Python literal conversion utilities.

use crate::e2e::escape::escape_python;

/// Convert a `serde_json::Value` to a Python literal string.
pub(super) fn json_to_python_literal(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "None".to_string(),
        serde_json::Value::Bool(true) => "True".to_string(),
        serde_json::Value::Bool(false) => "False".to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => python_string_literal(s),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_python_literal).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("\"{}\": {}", escape_python(k), json_to_python_literal(v)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

pub(super) fn value_to_python_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => python_string_literal(s),
        serde_json::Value::Bool(true) => "True".to_string(),
        serde_json::Value::Bool(false) => "False".to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "None".to_string(),
        other => python_string_literal(&other.to_string()),
    }
}

/// Produce a quoted Python string literal, choosing single or double quotes
/// to avoid unnecessary escaping (ruff Q003).
pub(super) fn python_string_literal(s: &str) -> String {
    if s.contains('"') && !s.contains('\'') {
        // Use single quotes to avoid escaping double quotes.
        let escaped = s
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
        format!("'{escaped}'")
    } else {
        format!("\"{}\"", escape_python(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_to_python_literal_null_returns_none() {
        assert_eq!(json_to_python_literal(&serde_json::Value::Null), "None");
    }

    #[test]
    fn json_to_python_literal_bool_true_returns_true() {
        assert_eq!(json_to_python_literal(&serde_json::Value::Bool(true)), "True");
    }

    #[test]
    fn python_string_literal_uses_single_quotes_when_has_double_quotes() {
        let out = python_string_literal("say \"hello\"");
        assert!(out.starts_with('\''), "got: {out}");
    }

    #[test]
    fn value_to_python_string_number_returns_number_string() {
        let v = serde_json::json!(42u64);
        assert_eq!(value_to_python_string(&v), "42");
    }
}
