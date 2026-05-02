//! Rust argument rendering helpers.

use crate::escape::rust_raw_string;

/// Render a single argument binding and expression for a Rust e2e test call.
///
/// Returns `(binding_lines, call_expression)`.
#[allow(clippy::too_many_arguments)]
pub fn render_rust_arg(
    name: &str,
    value: &serde_json::Value,
    arg_type: &str,
    optional: bool,
    module: &str,
    fixture_id: &str,
    mock_base_url: Option<&str>,
    owned: bool,
    element_type: Option<&str>,
) -> (Vec<String>, String) {
    if arg_type == "mock_url" {
        let lines = vec![format!(
            "let {name} = format!(\"{{}}/fixtures/{{}}\", std::env::var(\"MOCK_SERVER_URL\").expect(\"MOCK_SERVER_URL not set\"), \"{fixture_id}\");"
        )];
        return (lines, format!("&{name}"));
    }
    // When the arg is a base_url and a mock server is running, use the mock server URL.
    if arg_type == "base_url" {
        if let Some(url_expr) = mock_base_url {
            return (vec![], url_expr.to_string());
        }
        // No mock server: fall through to string handling below.
    }
    if arg_type == "handle" {
        // Generate a create_engine (or equivalent) call and pass the config.
        // If the fixture has input.config, serialize it as a json_object and pass it;
        // otherwise pass None.
        use heck::ToSnakeCase;
        let constructor_name = format!("create_{}", name.to_snake_case());
        let mut lines = Vec::new();
        if value.is_null() || value.is_object() && value.as_object().unwrap().is_empty() {
            lines.push(format!(
                "let {name} = {constructor_name}(None).expect(\"handle creation should succeed\");"
            ));
        } else {
            // Serialize the config JSON and deserialize at runtime.
            let json_literal = serde_json::to_string(value).unwrap_or_default();
            let escaped = json_literal.replace('\\', "\\\\").replace('"', "\\\"");
            lines.push(format!(
                "let {name}_config: CrawlConfig = serde_json::from_str(\"{escaped}\").expect(\"config should parse\");"
            ));
            lines.push(format!(
                "let {name} = {constructor_name}(Some({name}_config)).expect(\"handle creation should succeed\");"
            ));
        }
        return (lines, format!("&{name}"));
    }
    if arg_type == "json_object" {
        return render_json_object_arg(name, value, optional, owned, element_type, module);
    }
    if value.is_null() && !optional {
        // Required arg with no fixture value: use a language-appropriate default.
        let default_val = match arg_type {
            "string" => "String::new()".to_string(),
            "int" | "integer" => "0".to_string(),
            "float" | "number" => "0.0_f64".to_string(),
            "bool" | "boolean" => "false".to_string(),
            _ => "Default::default()".to_string(),
        };
        // String args are passed by reference in Rust.
        let expr = if arg_type == "string" {
            format!("&{name}")
        } else {
            name.to_string()
        };
        return (vec![format!("let {name} = {default_val};")], expr);
    }
    // Bytes args whose fixture value is a string are treated as relative file paths into
    // test_documents/. Emit `std::fs::read(...)` to load the binary content at test-run
    // time instead of passing the path string as inline bytes via `.as_bytes()`.
    // This matches the upstream behaviour introduced in alef-e2e commit 9133eb3b.
    if arg_type == "bytes" {
        if let serde_json::Value::String(path_str) = value {
            // File-path value: load via std::fs::read at test-run time.
            let binding = format!(
                "let {name} = std::fs::read(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../test_documents/{path_str}\")).expect(\"test_documents/{path_str} must exist\");"
            );
            let call_expr = if owned { name.to_string() } else { format!("&{name}") };
            return (vec![binding], call_expr);
        }
        // Null optional bytes → None binding.
        if value.is_null() && optional {
            return (
                vec![format!("let {name}: Option<Vec<u8>> = None;")],
                format!("{name}.as_deref().map(|v| v.as_slice())"),
            );
        }
    }

    let literal = json_to_rust_literal(value, arg_type);
    // String args are raw string literals (`r#"..."#`) — already `&str`, no extra `&` needed.
    // Bytes args without a file-path value are passed by reference using `.as_bytes()` below.
    let optional_expr = |n: &str| {
        if arg_type == "string" {
            format!("{n}.as_deref()")
        } else if arg_type == "bytes" {
            format!("{n}.as_deref().map(|v| v.as_slice())")
        } else {
            // Owned numeric / bool / generic: pass the Option<T> by value.
            // Function signature shape `Option<T>` matches without `.as_ref()`,
            // which would produce `Option<&T>` and fail to coerce.
            n.to_string()
        }
    };
    let expr = |n: &str| {
        if arg_type == "bytes" {
            format!("{n}.as_bytes()")
        } else if arg_type == "string" && owned {
            // Owned string: caller expects `String` by value, not `&str`.
            format!("{n}.to_string()")
        } else {
            n.to_string()
        }
    };
    if optional && value.is_null() {
        let none_decl = match arg_type {
            "string" => format!("let {name}: Option<String> = None;"),
            "bytes" => format!("let {name}: Option<Vec<u8>> = None;"),
            _ => format!("let {name} = None;"),
        };
        (vec![none_decl], optional_expr(name))
    } else if optional {
        (vec![format!("let {name} = Some({literal});")], optional_expr(name))
    } else {
        (vec![format!("let {name} = {literal};")], expr(name))
    }
}

/// Render a `json_object` argument: serialize the fixture JSON as a `serde_json::json!` literal
/// and deserialize it through serde at runtime. Type inference from the function signature
/// determines the concrete type, keeping the generator generic.
///
/// `owned` — when true the binding is passed by value (no leading `&`); use for `Vec<T>` params.
/// `element_type` — when set, emits `Vec<element_type>` annotation to satisfy type inference for
///   `&[T]` parameters where `serde_json::from_value` cannot resolve the unsized slice type.
fn render_json_object_arg(
    name: &str,
    value: &serde_json::Value,
    optional: bool,
    owned: bool,
    element_type: Option<&str>,
    _module: &str,
) -> (Vec<String>, String) {
    // Owned params (Vec<T>) are passed by value; ref params (most configs) use &.
    let pass_by_ref = !owned;

    if value.is_null() && optional {
        // Use Default::default() — Rust functions take &T (or T for owned), not Option<T>.
        let expr = if pass_by_ref {
            format!("&{name}")
        } else {
            name.to_string()
        };
        return (vec![format!("let {name} = Default::default();")], expr);
    }

    // Fixture keys are camelCase; the Rust ConversionOptions type uses snake_case serde.
    // Normalize keys before building the json! literal so deserialization succeeds.
    let normalized = super::super::normalize_json_keys_to_snake_case(value);
    // Build the json! macro invocation from the fixture object.
    let json_literal = json_value_to_macro_literal(&normalized);
    let mut lines = Vec::new();
    lines.push(format!("let {name}_json = serde_json::json!({json_literal});"));

    // When an explicit element type is given, annotate with Vec<T> so that
    // serde_json::from_value can infer the element type for &[T] parameters (A4 fix).
    let deser_expr = if let Some(elem) = element_type {
        format!("serde_json::from_value::<Vec<{elem}>>({name}_json).unwrap()")
    } else {
        format!("serde_json::from_value({name}_json).unwrap()")
    };

    // A1 fix: always deser as T (never wrap in Some()); optional non-null args target
    // &T not &Option<T>. Pass as &T (ref) or T (owned) depending on the `owned` flag.
    lines.push(format!("let {name} = {deser_expr};"));
    let expr = if pass_by_ref {
        format!("&{name}")
    } else {
        name.to_string()
    };
    (lines, expr)
}

/// Convert a `serde_json::Value` into a string suitable for the `serde_json::json!()` macro.
pub fn json_value_to_macro_literal(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => format!("{b}"),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => {
            let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{escaped}\"")
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_value_to_macro_literal).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(obj) => {
            let entries: Vec<String> = obj
                .iter()
                .map(|(k, v)| {
                    let escaped_key = k.replace('\\', "\\\\").replace('"', "\\\"");
                    format!("\"{escaped_key}\": {}", json_value_to_macro_literal(v))
                })
                .collect();
            format!("{{{}}}", entries.join(", "))
        }
    }
}

pub fn json_to_rust_literal(value: &serde_json::Value, arg_type: &str) -> String {
    match value {
        serde_json::Value::Null => "None".to_string(),
        serde_json::Value::Bool(b) => format!("{b}"),
        serde_json::Value::Number(n) => {
            if arg_type.contains("float") || arg_type.contains("f64") || arg_type.contains("f32") {
                if let Some(f) = n.as_f64() {
                    return format!("{f}_f64");
                }
            }
            n.to_string()
        }
        serde_json::Value::String(s) => rust_raw_string(s),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            let literal = rust_raw_string(&json_str);
            format!("serde_json::from_str({literal}).unwrap()")
        }
    }
}

/// Resolve the visitor trait name based on module.
pub fn resolve_visitor_trait(module: &str) -> String {
    // For html_to_markdown modules, use HtmlVisitor
    if module.contains("html_to_markdown") {
        "HtmlVisitor".to_string()
    } else {
        // Default fallback for other modules
        "Visitor".to_string()
    }
}

/// Emit a Rust visitor method for a callback action.
///
/// The parameter type list mirrors the `HtmlVisitor` trait in
/// `kreuzberg-dev/html-to-markdown`. Param names are bound to `_` because the
/// generated visitor body never references them — the body always returns a
/// fixed `VisitResult` variant — so we'd otherwise hit `unused_variables`
/// warnings that fail prek's `cargo clippy -D warnings` hook.
pub fn emit_rust_visitor_method(out: &mut String, method_name: &str, action: &crate::fixture::CallbackAction) {
    use std::fmt::Write as FmtWrite;
    // Each entry: parameters typed exactly as `HtmlVisitor` expects them,
    // bound to `_` patterns so the generated body needn't introduce unused
    // bindings. Receiver is `&mut self` to match the trait.
    let params = match method_name {
        "visit_link" => "_: &NodeContext, _: &str, _: &str, _: &str",
        "visit_image" => "_: &NodeContext, _: &str, _: &str, _: &str",
        "visit_heading" => "_: &NodeContext, _: u8, _: &str, _: Option<&str>",
        "visit_code_block" => "_: &NodeContext, _: Option<&str>, _: &str",
        "visit_code_inline"
        | "visit_strong"
        | "visit_emphasis"
        | "visit_strikethrough"
        | "visit_underline"
        | "visit_subscript"
        | "visit_superscript"
        | "visit_mark"
        | "visit_button"
        | "visit_summary"
        | "visit_figcaption"
        | "visit_definition_term"
        | "visit_definition_description" => "_: &NodeContext, _: &str",
        "visit_text" => "_: &NodeContext, _: &str",
        "visit_list_item" => "_: &NodeContext, _: bool, _: &str, _: &str",
        "visit_blockquote" => "_: &NodeContext, _: &str, _: u32",
        "visit_table_row" => "_: &NodeContext, _: &[String], _: bool",
        "visit_custom_element" => "_: &NodeContext, _: &str, _: &str",
        "visit_form" => "_: &NodeContext, _: &str, _: &str",
        "visit_input" => "_: &NodeContext, _: &str, _: &str, _: &str",
        "visit_audio" | "visit_video" | "visit_iframe" => "_: &NodeContext, _: &str",
        "visit_details" => "_: &NodeContext, _: bool",
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => {
            "_: &NodeContext, _: &str"
        }
        "visit_list_start" => "_: &NodeContext, _: bool",
        "visit_list_end" => "_: &NodeContext, _: bool, _: &str",
        _ => "_: &NodeContext",
    };

    let _ = writeln!(out, "        fn {method_name}(&mut self, {params}) -> VisitResult {{");
    match action {
        crate::fixture::CallbackAction::Skip => {
            let _ = writeln!(out, "            VisitResult::Skip");
        }
        crate::fixture::CallbackAction::Continue => {
            let _ = writeln!(out, "            VisitResult::Continue");
        }
        crate::fixture::CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "            VisitResult::PreserveHtml");
        }
        crate::fixture::CallbackAction::Custom { output } => {
            let escaped = crate::escape::escape_rust(output);
            let _ = writeln!(out, "            VisitResult::Custom(\"{escaped}\".to_string())");
        }
        crate::fixture::CallbackAction::CustomTemplate { template } => {
            let escaped = crate::escape::escape_rust(template);
            let _ = writeln!(out, "            VisitResult::Custom(format!(\"{escaped}\"))");
        }
    }
    let _ = writeln!(out, "        }}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_value_to_macro_literal_null() {
        let v = serde_json::Value::Null;
        assert_eq!(json_value_to_macro_literal(&v), "null");
    }

    #[test]
    fn json_value_to_macro_literal_string_escapes_quotes() {
        let v = serde_json::Value::String("hello \"world\"".to_string());
        let out = json_value_to_macro_literal(&v);
        assert!(out.contains("\\\""));
    }

    #[test]
    fn json_to_rust_literal_null_returns_none() {
        let out = json_to_rust_literal(&serde_json::Value::Null, "string");
        assert_eq!(out, "None");
    }

    #[test]
    fn resolve_visitor_trait_html_to_markdown() {
        assert_eq!(resolve_visitor_trait("html_to_markdown"), "HtmlVisitor");
        assert_eq!(resolve_visitor_trait("other_module"), "Visitor");
    }
}
