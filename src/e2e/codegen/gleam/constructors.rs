use crate::e2e::escape::escape_gleam;

/// Render a single Gleam record-constructor call for one item of a
/// `json_object` list arg, driven by a `[crates.gleam.element_constructors]`
/// entry. Each field is dispatched by its `kind`:
///
/// * `file_path` - emits a Gleam string literal; relative paths are prefixed
///   with `test_documents_path` so they resolve from the e2e working dir.
/// * `byte_array` - emits a Gleam BitArray literal `<<n1, n2, ...>>` from a
///   JSON array of unsigned integers.
/// * `string` - emits a Gleam string literal; missing/null falls back to
///   the field's `default` (or `""`).
/// * `literal` - emits the field's `value` verbatim.
pub(super) fn render_gleam_element_constructor(
    item: &serde_json::Value,
    recipe: &crate::core::config::GleamElementConstructor,
    test_documents_path: &str,
) -> String {
    let mut field_exprs: Vec<String> = Vec::with_capacity(recipe.fields.len());
    for field in &recipe.fields {
        let expr = match field.kind.as_str() {
            "file_path" => {
                let json_field = field.json_field.as_deref().unwrap_or("");
                let path = item.get(json_field).and_then(|v| v.as_str()).unwrap_or("");
                let full = if path.starts_with('/') {
                    path.to_string()
                } else {
                    format!("{test_documents_path}/{path}")
                };
                format!("\"{}\"", escape_gleam(&full))
            }
            "byte_array" => {
                let json_field = field.json_field.as_deref().unwrap_or("");
                let bytes: Vec<String> = item
                    .get(json_field)
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().map(|b| b.as_u64().unwrap_or(0).to_string()).collect())
                    .unwrap_or_default();
                if bytes.is_empty() {
                    "<<>>".to_string()
                } else {
                    format!("<<{}>>", bytes.join(", "))
                }
            }
            "string" => {
                let json_field = field.json_field.as_deref().unwrap_or("");
                let value = item
                    .get(json_field)
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or_else(|| field.default.clone())
                    .unwrap_or_default();
                format!("\"{}\"", escape_gleam(&value))
            }
            "literal" => field.value.clone().unwrap_or_default(),
            other => {
                // Unknown kind — fall back to a verbatim literal of the value
                // field if present, else an empty string. Surfacing the
                // unsupported kind in the generated code makes the error
                // visible at compile-time rather than failing silently.
                field
                    .value
                    .clone()
                    .unwrap_or_else(|| format!("\"<unsupported kind: {other}>\""))
            }
        };
        field_exprs.push(format!("{}: {}", field.gleam_field, expr));
    }
    format!("{}({})", recipe.constructor, field_exprs.join(", "))
}
