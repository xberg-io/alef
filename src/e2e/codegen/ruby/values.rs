//! Ruby e2e value/literal helpers.

use heck::ToUpperCamelCase;

/// Convert a module path (e.g., "demo_markup") to Ruby PascalCase module name
/// (e.g., "DemoMarkup").
pub(super) fn ruby_module_name(module_path: &str) -> String {
    module_path.to_upper_camel_case()
}

/// Convert a `serde_json::Value` to a Ruby literal string, preferring single quotes.
pub(super) fn json_to_ruby(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => crate::e2e::escape::ruby_string_literal(s),
        serde_json::Value::Bool(true) => "true".to_string(),
        serde_json::Value::Bool(false) => "false".to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "nil".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_ruby).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{} => {}", crate::e2e::escape::ruby_string_literal(k), json_to_ruby(v)))
                .collect();
            format!("{{ {} }}", items.join(", "))
        }
    }
}

/// Classify a fixture string value that maps to a `bytes` argument.
///
/// Returns true if the value looks like a file path (e.g. "pdf/fake_memo.pdf").
/// File paths have the pattern: alphanumeric/something.extension
pub(super) fn is_file_path(s: &str) -> bool {
    if s.starts_with('<') || s.starts_with('{') || s.starts_with('[') || s.contains(' ') {
        return false;
    }

    let first = s.chars().next().unwrap_or('\0');
    if first.is_ascii_alphanumeric() || first == '_' {
        if let Some(slash_pos) = s.find('/') {
            if slash_pos > 0 {
                let after_slash = &s[slash_pos + 1..];
                if after_slash.contains('.') && !after_slash.is_empty() {
                    return true;
                }
            }
        }
    }

    false
}

/// Check if a string looks like base64-encoded data.
///
/// If it's not a file path or inline text, assume it's base64.
pub(super) fn is_base64(s: &str) -> bool {
    if s.starts_with('<') || s.starts_with('{') || s.starts_with('[') || s.contains(' ') {
        return false;
    }

    if is_file_path(s) {
        return false;
    }

    true
}
