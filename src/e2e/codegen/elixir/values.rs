use crate::e2e::escape::escape_elixir;

/// Convert a category name to an Elixir module-safe PascalCase name.
pub(super) fn elixir_module_name(category: &str) -> String {
    use heck::ToUpperCamelCase;
    category.to_upper_camel_case()
}

/// Convert a `serde_json::Value` to an Elixir literal string.
pub(super) fn json_to_elixir(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_elixir(s)),
        serde_json::Value::Bool(true) => "true".to_string(),
        serde_json::Value::Bool(false) => "false".to_string(),
        serde_json::Value::Number(n) => {
            // Elixir requires floats to have a decimal point and does not accept
            // `e+N` exponent notation. Strip the `+` and ensure there is a decimal
            // point before any `e` exponent marker (e.g. `1e-10` -> `1.0e-10`).
            let s = n.to_string().replace("e+", "e");
            if s.contains('e') && !s.contains('.') {
                // Insert `.0` before the `e` so Elixir treats this as a float.
                s.replacen('e', ".0e", 1)
            } else {
                s
            }
        }
        serde_json::Value::Null => "nil".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_elixir).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("\"{}\" => {}", escape_elixir(k), json_to_elixir(v)))
                .collect();
            format!("%{{{}}}", entries.join(", "))
        }
    }
}
