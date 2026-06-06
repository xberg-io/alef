use crate::e2e::escape::escape_gleam;

/// Return a sensible Gleam default value for `option.unwrap(default)` based
/// on the type inferred from the JSON expected value string.
pub(super) fn default_gleam_value_for_optional(gleam_val: &str) -> &'static str {
    if gleam_val.starts_with('"') {
        "\"\""
    } else if gleam_val == "True" || gleam_val == "False" {
        "False"
    } else if gleam_val.contains('.') {
        "0.0"
    } else {
        "0"
    }
}

/// Convert a `serde_json::Value` to a Gleam literal string.
pub(super) fn json_to_gleam(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_gleam(s)),
        serde_json::Value::Bool(b) => {
            if *b {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "Nil".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_gleam).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_gleam(&json_str))
        }
    }
}
