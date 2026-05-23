//! JSON-to-JavaScript literal conversion utilities.

use crate::e2e::escape::{escape_js, expand_fixture_templates};

/// Convert a `serde_json::Value` to a JavaScript literal string.
pub(super) fn json_to_js(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => {
            let expanded = expand_fixture_templates(s);
            format!("\"{}\"", escape_js(&expanded))
        }
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => {
            // For integers outside JS safe range, emit as string to avoid precision loss.
            if let Some(i) = n.as_i64() {
                if !(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&i) {
                    return format!("Number(\"{i}\")");
                }
            }
            if let Some(u) = n.as_u64() {
                if u > 9_007_199_254_740_991 {
                    return format!("Number(\"{u}\")");
                }
            }
            n.to_string()
        }
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_js).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    // Quote keys that aren't valid JS identifiers (contain hyphens, spaces, etc.)
                    let key = if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
                        && !k.starts_with(|c: char| c.is_ascii_digit())
                    {
                        k.clone()
                    } else {
                        format!("\"{}\"", escape_js(k))
                    };
                    format!("{key}: {}", json_to_js(v))
                })
                .collect();
            format!("{{ {} }}", entries.join(", "))
        }
    }
}

/// Convert a `serde_json::Value` to a JavaScript literal string with camelCase object keys.
///
/// NAPI-RS bindings use camelCase for JavaScript field names. This variant converts
/// snake_case object keys (as written in fixture JSON) to camelCase so that the
/// generated config objects match the NAPI binding's expected field names.
pub(super) fn json_to_js_camel(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let camel_key = snake_to_camel(k);
                    // Quote keys that aren't valid JS identifiers.
                    let key = if camel_key
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
                        && !camel_key.starts_with(|c: char| c.is_ascii_digit())
                    {
                        camel_key.clone()
                    } else {
                        format!("\"{}\"", escape_js(&camel_key))
                    };
                    format!("{key}: {}", json_to_js_camel(v))
                })
                .collect();
            format!("{{ {} }}", entries.join(", "))
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_js_camel).collect();
            format!("[{}]", items.join(", "))
        }
        // Scalars and null delegate to the standard converter.
        other => json_to_js(other),
    }
}

/// Convert a snake_case string to camelCase.
pub(super) fn snake_to_camel(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = false;
    for ch in s.chars() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_to_js_string_escapes_double_quotes() {
        let val = serde_json::Value::String("say \"hello\"".to_string());
        let out = json_to_js(&val);
        assert!(out.contains("\\\""), "got: {out}");
    }

    #[test]
    fn json_to_js_null_returns_null_literal() {
        assert_eq!(json_to_js(&serde_json::Value::Null), "null");
    }

    #[test]
    fn snake_to_camel_converts_underscores() {
        assert_eq!(snake_to_camel("hello_world"), "helloWorld");
        assert_eq!(snake_to_camel("no_underscores"), "noUnderscores");
        assert_eq!(snake_to_camel("already"), "already");
    }

    #[test]
    fn json_to_js_camel_converts_object_keys() {
        let val = serde_json::json!({ "my_field": 1 });
        let out = json_to_js_camel(&val);
        assert!(out.contains("myField"), "got: {out}");
        assert!(!out.contains("my_field"), "got: {out}");
    }
}
