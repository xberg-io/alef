//! JSON-to-Python literal conversion utilities.

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
                .map(|(k, v)| format!("{}: {}", python_string_literal(k), json_to_python_literal(v)))
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
/// to avoid unnecessary escaping (ruff Q003). Non-ASCII characters are emitted as
/// `\u`/`\U` escapes so no ambiguous-unicode literal (ruff RUF001) reaches the source.
pub(super) fn python_string_literal(s: &str) -> String {
    let quote = if s.contains('"') && !s.contains('\'') {
        '\''
    } else {
        '"'
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c if c.is_ascii() => out.push(c),
            c if (c as u32) <= 0xFFFF => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push_str(&format!("\\U{:08x}", c as u32)),
        }
    }
    out.push(quote);
    out
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
    fn python_string_literal_json_payload_has_no_escaped_quotes() {
        let out = python_string_literal(r#"{"completion_window":"24h"}"#);
        assert_eq!(out, r#"'{"completion_window":"24h"}'"#);
    }

    #[test]
    fn python_string_literal_mixed_quotes_keeps_double_quotes_with_escapes() {
        assert_eq!(python_string_literal(r#"a"b'c"#), r#""a\"b'c""#);
    }

    #[test]
    fn python_string_literal_escapes_non_ascii_as_unicode_escapes() {
        assert_eq!(python_string_literal("\u{203a} \u{20ac}"), r#""\u203a \u20ac""#);
        assert_eq!(python_string_literal("\u{1f600}"), r#""\U0001f600""#);
    }

    #[test]
    fn python_string_literal_escapes_control_chars_in_single_quoted_form() {
        assert_eq!(python_string_literal("\"\u{1}"), r#"'"\x01'"#);
    }

    #[test]
    fn value_to_python_string_number_returns_number_string() {
        let v = serde_json::json!(42u64);
        assert_eq!(value_to_python_string(&v), "42");
    }
}
