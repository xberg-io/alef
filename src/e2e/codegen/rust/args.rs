//! Rust argument rendering helpers.

use crate::e2e::escape::rust_raw_string;
use crate::e2e::fixture::FixtureDocsFileInput;

pub(crate) fn resolve_handle_config_type(
    arg: &crate::e2e::config::ArgMapping,
    options_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<String> {
    if arg.arg_type != "handle" {
        return None;
    }
    options_type.map(str::to_string).or_else(|| {
        use heck::ToUpperCamelCase;
        let candidate = format!("{}Config", arg.name.to_upper_camel_case());
        type_defs.iter().any(|ty| ty.name == candidate).then_some(candidate)
    })
}

/// Render a single argument binding and expression for a Rust e2e test call.
///
/// Returns `(binding_lines, call_expression)`.
///
/// `test_documents_dir` is the configured fixture-binary directory name (see
/// [`E2eConfig::test_documents_dir`]). It is concatenated at compile time with
/// the `CARGO_MANIFEST_DIR` so that fixture-relative paths resolve from any
/// `cargo` invocation cwd.
///
/// `is_error_context` — when `true` the fixture asserts an error outcome.
/// For `handle` args this changes `.expect(...)` to a bare `Result` binding
/// so engine-creation failures can be propagated to the final assertion.
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
    test_documents_dir: &str,
    is_error_context: bool,
    handle_config_type: Option<&str>,
    docs_files: &[FixtureDocsFileInput],
    vec_inner_is_ref: bool,
    preserve_input_urls: bool,
) -> (Vec<String>, String) {
    if arg_type == "mock_url" {
        if let Some(url) = crate::e2e::codegen::preserved_url_literal(preserve_input_urls, value) {
            let lines = vec![format!("let {name} = {}.to_string();", rust_raw_string(url))];
            return (lines, format!("&{name}"));
        }
        // Prefer the per-fixture `MOCK_SERVER_<FIXTURE_ID>` env var when set (host-root
        // fixtures get their own listener — robots.txt and sitemap.xml must live at the
        // host root). Fall back to `MOCK_SERVER_URL/fixtures/<id>` for the common case.
        let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
        let lines = vec![format!(
            "let {name} = std::env::var(\"{env_key}\").unwrap_or_else(|_| {{ let _ = common::mock_server_url(); std::env::var(\"{env_key}\").unwrap_or_else(|_| format!(\"{{}}/fixtures/{{}}\", common::mock_server_url(), \"{fixture_id}\")) }});"
        )];
        return (lines, format!("&{name}"));
    }
    if arg_type == "mock_url_list" {
        if let Some(urls) = crate::e2e::codegen::preserved_url_list(preserve_input_urls, value) {
            let literals: Vec<String> = urls.iter().map(|url| rust_raw_string(url)).collect();
            let lines = vec![format!(
                "let {name}: Vec<String> = [{}].into_iter().map(str::to_string).collect();",
                literals.join(", ")
            )];
            let expr = if owned { name.to_string() } else { format!("&{name}") };
            return (lines, expr);
        }
        // Vec<String> of URLs: each element is either a bare path (`/seed1`) — prefixed
        // with the per-fixture mock-server URL at runtime — or an absolute URL kept as-is.
        // Mirrors the `mock_url` resolution: `MOCK_SERVER_<FIXTURE_ID>` first, then
        // `MOCK_SERVER_URL/fixtures/<id>`.
        let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
        let paths: Vec<String> = if let Some(arr) = value.as_array() {
            arr.iter().filter_map(|v| v.as_str().map(rust_raw_string)).collect()
        } else {
            Vec::new()
        };
        let paths_literal = paths.join(", ");
        let mut lines = Vec::new();
        lines.push(format!(
            "let {name}_base = std::env::var(\"{env_key}\").unwrap_or_else(|_| {{ let _ = common::mock_server_url(); std::env::var(\"{env_key}\").unwrap_or_else(|_| format!(\"{{}}/fixtures/{{}}\", common::mock_server_url(), \"{fixture_id}\")) }});"
        ));
        lines.push(format!(
            "let {name}: Vec<String> = [{paths_literal}].into_iter().map(|p: &str| if p.starts_with(\"http\") {{ p.to_string() }} else {{ format!(\"{{}}{{}}\", {name}_base, p) }}).collect();"
        ));
        let expr = if owned { name.to_string() } else { format!("&{name}") };
        return (lines, expr);
    }
    // When the arg is a base_url and a mock server is running, use the mock server URL.
    if arg_type == "base_url"
        && let Some(url_expr) = mock_base_url
    {
        return (vec![], url_expr.to_string());
    }
    // No mock server: fall through to string handling below.
    if arg_type == "handle" {
        // Generate a create_engine (or equivalent) call and pass the config.
        // If the fixture has input.config, serialize it as a json_object and pass it;
        // otherwise pass None.
        use heck::ToSnakeCase;
        let constructor_name = format!("create_{}", name.to_snake_case());
        let mut lines = Vec::new();
        if is_error_context {
            // In error context the engine creation itself may legitimately fail (e.g.
            // invalid config). Bind to a Result so the caller can propagate the error
            // to the final assertion instead of panicking with .expect().
            if value.is_null() || value.is_object() && value.as_object().unwrap().is_empty() {
                lines.push(format!("let {name}_result = {constructor_name}(None);"));
            } else {
                let json_literal = serde_json::to_string(value).unwrap_or_default();
                let escaped = json_literal.replace('\\', "\\\\").replace('"', "\\\"");
                let type_annotation = handle_config_type.map_or(String::new(), |ty| format!(": {ty}"));
                lines.push(format!(
                    "let {name}_config{type_annotation} = serde_json::from_str(\"{escaped}\").expect(\"config should parse\");"
                ));
                lines.push(format!("let {name}_result = {constructor_name}(Some({name}_config));"));
            }
            // The call expression is a sentinel that the test_file emitter will detect
            // and wrap in a match; we return the result binding name as the expression.
            return (lines, format!("{name}_result"));
        }
        if value.is_null() || value.is_object() && value.as_object().unwrap().is_empty() {
            lines.push(format!(
                "let {name} = {constructor_name}(None).expect(\"handle creation should succeed\");"
            ));
        } else {
            // Serialize the config JSON and deserialize at runtime.
            let json_literal = serde_json::to_string(value).unwrap_or_default();
            let escaped = json_literal.replace('\\', "\\\\").replace('"', "\\\"");
            let type_annotation = handle_config_type.map_or(String::new(), |ty| format!(": {ty}"));
            lines.push(format!(
                "let {name}_config{type_annotation} = serde_json::from_str(\"{escaped}\").expect(\"config should parse\");"
            ));
            lines.push(format!(
                "let {name} = {constructor_name}(Some({name}_config)).expect(\"handle creation should succeed\");"
            ));
        }
        return (lines, format!("&{name}"));
    }
    if arg_type == "json_object" {
        return render_json_object_arg(
            name,
            value,
            optional,
            owned,
            element_type,
            module,
            fixture_id,
            vec_inner_is_ref,
            docs_files,
        );
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
            let binding = if docs_files.iter().any(|file| file.field.is_empty()) {
                crate::e2e::template_env::render(
                    "rust/docs_file_read.rs.jinja",
                    crate::alef_context! { name => name, path => rust_raw_string(path_str) },
                )
                .trim_end()
                .to_string()
            } else {
                format!(
                    "let {name} = std::fs::read(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../../{test_documents_dir}/{path_str}\")).expect(\"{test_documents_dir}/{path_str} must exist\");"
                )
            };
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
    // file_path args are fixture-relative paths into the repo-root `test_documents/`
    // directory; resolve to a `&'static str` absolute path at compile time so the test
    // can run from any cwd without depending on the current working directory.
    if arg_type == "file_path"
        && let serde_json::Value::String(path_str) = value
    {
        let binding = format!(
            "let {name}: &str = concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../../{test_documents_dir}/\", \"{path_str}\");"
        );
        return (vec![binding], name.to_string());
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
/// `vec_inner_is_ref` — when true and `element_type = "String"`, emits an extra binding that
///   converts `Vec<String>` to `Vec<&str>` so the slice coerces to `&[&str]` as required by the
///   Rust core. This mirrors the `vec_inner_is_ref` flag on `ParamDef` in the binding shims.
#[allow(clippy::too_many_arguments)]
fn render_json_object_arg(
    name: &str,
    value: &serde_json::Value,
    optional: bool,
    owned: bool,
    element_type: Option<&str>,
    _module: &str,
    fixture_id: &str,
    vec_inner_is_ref: bool,
    docs_files: &[FixtureDocsFileInput],
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

    // Rust core uses snake_case serde — transform fixture keys to snake_case before
    // serializing so deserialization through the typed `from_str` succeeds.
    let normalized = super::super::transform_json_keys_for_language(value, "snake_case");
    // Embed the fixture object as a raw JSON string literal and parse it at runtime.
    // Using `serde_json::from_str` (over the previous `serde_json::json!` macro) avoids
    // the macro's recursion limit, which is reached when a fixture's `json_object` arg
    // contains more than ~100 array elements (e.g. `interact_max_actions_exceeded`).
    let json_text = serde_json::to_string(&normalized).unwrap_or_else(|_| "null".to_string());
    let json_literal = rust_raw_string(&json_text);
    let mut lines = Vec::new();
    if super::super::value_contains_mock_url_placeholder(&normalized) {
        let env_key = super::super::mock_url_env_key(fixture_id);
        lines.push(format!(
            "let {name}_mock_base_url = std::env::var(\"{env_key}\").unwrap_or_else(|_| {{ let _ = common::mock_server_url(); std::env::var(\"{env_key}\").unwrap_or_else(|_| format!(\"{{}}/fixtures/{{}}\", common::mock_server_url(), \"{fixture_id}\")) }});"
        ));
        lines.push(format!(
            "let {name}_json_text = {json_literal}.replace(\"{}\", &{name}_mock_base_url);",
            super::super::MOCK_URL_PLACEHOLDER
        ));
        let mutability = if docs_files.is_empty() { "" } else { "mut " };
        lines.push(format!(
            "let {mutability}{name}_json: serde_json::Value = serde_json::from_str(&{name}_json_text).unwrap();"
        ));
    } else {
        let mutability = if docs_files.is_empty() { "" } else { "mut " };
        lines.push(format!(
            "let {mutability}{name}_json: serde_json::Value = serde_json::from_str({json_literal}).unwrap();"
        ));
    }
    for (index, file) in docs_files.iter().enumerate() {
        let binding = format!("{name}_file_{index}");
        lines.extend(
            crate::e2e::template_env::render(
                "rust/docs_json_file.rs.jinja",
                crate::alef_context! {
                    binding => binding,
                    path => rust_raw_string(&file.path),
                    json_name => format!("{name}_json"),
                    pointer => rust_raw_string(&file.field),
                },
            )
            .lines()
            .map(str::to_string),
        );
    }

    // When an explicit element type is given, annotate with Vec<T> so that
    // serde_json::from_value can infer either the single DTO type or the element
    // type for Vec<T> / &[T] parameters.
    let deser_expr = if let Some(elem) = element_type {
        if normalized.is_array() {
            format!("serde_json::from_value::<Vec<{elem}>>({name}_json).unwrap()")
        } else {
            format!("serde_json::from_value::<{elem}>({name}_json).unwrap()")
        }
    } else {
        format!("serde_json::from_value({name}_json).unwrap()")
    };

    // A1 fix: always deser as T (never wrap in Some()); optional non-null args target
    // &T not &Option<T>. Pass as &T (ref) or T (owned) depending on the `owned` flag.
    lines.push(format!("let {name} = {deser_expr};"));
    // When vec_inner_is_ref is set and element_type is "String", the Rust core parameter is
    // &[&str] rather than &[String]. Emit a conversion binding so the slice coerces correctly.
    if vec_inner_is_ref && matches!(element_type, Some("String")) {
        let refs_name = format!("{name}_refs");
        lines.push(format!(
            "let {refs_name}: Vec<&str> = {name}.iter().map(String::as_str).collect();"
        ));
        return (lines, format!("&{refs_name}"));
    }
    let expr = if pass_by_ref {
        format!("&{name}")
    } else {
        name.to_string()
    };
    (lines, expr)
}

pub fn json_to_rust_literal(value: &serde_json::Value, arg_type: &str) -> String {
    match value {
        serde_json::Value::Null => "None".to_string(),
        serde_json::Value::Bool(b) => format!("{b}"),
        serde_json::Value::Number(n) => {
            if (arg_type.contains("float") || arg_type.contains("f64") || arg_type.contains("f32"))
                && let Some(f) = n.as_f64()
            {
                return format!("{f}_f64");
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

/// Resolve the visitor trait name from the Rust e2e call override config.
///
/// Returns `Some(trait_name)` when `visitor_trait` is configured in the Rust
/// override, or `None` when unconfigured. Callers must treat `None` as a
/// codegen error when a fixture declares a `visitor` block.
pub fn resolve_visitor_trait(rust_override: Option<&crate::e2e::config::CallOverride>) -> Option<String> {
    rust_override.and_then(|o| o.visitor_trait.clone())
}

/// Emit a Rust visitor method for a callback action.
///
/// The parameter type list mirrors the configured visitor trait shape used by
/// visitor fixtures. For non-template actions params are bound
/// to `_name` patterns so the generated body needn't introduce unused bindings
/// (clippy `-D warnings` would otherwise fire). For `CustomTemplate` actions
/// the template string may reference named variables via `{name}` interpolation,
/// so those params are exposed with their real names and any `Option<T>` ones
/// that appear in the template are unwrapped with `.unwrap_or_default()`.
pub fn emit_rust_visitor_method(out: &mut String, method_name: &str, action: &crate::e2e::fixture::CallbackAction) {
    use std::fmt::Write as FmtWrite;

    // Each method entry: list of (name, type_str) pairs, excluding `&mut self`.
    // Types match the visitor-special trait shape used by current e2e fixtures.
    // `_ctx` is always first; subsequent params vary by method.
    let raw_params: &[(&str, &str)] = match method_name {
        "visit_link" => &[
            ("ctx", "&NodeContext"),
            ("href", "&str"),
            ("text", "&str"),
            ("title", "Option<&str>"),
        ],
        "visit_image" => &[
            ("ctx", "&NodeContext"),
            ("src", "&str"),
            ("alt", "&str"),
            ("title", "Option<&str>"),
        ],
        "visit_heading" => &[
            ("ctx", "&NodeContext"),
            ("level", "u32"),
            ("text", "&str"),
            ("id", "Option<&str>"),
        ],
        "visit_code_block" => &[("ctx", "&NodeContext"), ("lang", "Option<&str>"), ("code", "&str")],
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
        | "visit_definition_description" => &[("ctx", "&NodeContext"), ("text", "&str")],
        "visit_text" => &[("ctx", "&NodeContext"), ("text", "&str")],
        "visit_list_item" => &[
            ("ctx", "&NodeContext"),
            ("ordered", "bool"),
            ("marker", "&str"),
            ("text", "&str"),
        ],
        "visit_blockquote" => &[("ctx", "&NodeContext"), ("content", "&str"), ("depth", "usize")],
        "visit_table_row" => &[("ctx", "&NodeContext"), ("cells", "&[String]"), ("is_header", "bool")],
        "visit_custom_element" => &[("ctx", "&NodeContext"), ("tag_name", "&str"), ("html", "&str")],
        "visit_form" => &[
            ("ctx", "&NodeContext"),
            ("action", "Option<&str>"),
            ("method", "Option<&str>"),
        ],
        "visit_input" => &[
            ("ctx", "&NodeContext"),
            ("input_type", "&str"),
            ("name", "Option<&str>"),
            ("value", "Option<&str>"),
        ],
        "visit_audio" | "visit_video" | "visit_iframe" => &[("ctx", "&NodeContext"), ("src", "Option<&str>")],
        "visit_details" => &[("ctx", "&NodeContext"), ("open", "bool")],
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => {
            &[("ctx", "&NodeContext"), ("output", "&str")]
        }
        "visit_list_start" => &[("ctx", "&NodeContext"), ("ordered", "bool")],
        "visit_list_end" => &[("ctx", "&NodeContext"), ("ordered", "bool"), ("output", "&str")],
        _ => &[("ctx", "&NodeContext")],
    };

    let is_template = matches!(action, crate::e2e::fixture::CallbackAction::CustomTemplate { .. });

    // Determine which names the template references (only relevant for CustomTemplate).
    let template_vars: std::collections::HashSet<String> =
        if let crate::e2e::fixture::CallbackAction::CustomTemplate { template, .. } = action {
            // Extract `{name}` patterns from the template string.
            let mut vars = std::collections::HashSet::new();
            let mut chars = template.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '{' {
                    let mut var = String::new();
                    for inner in chars.by_ref() {
                        if inner == '}' {
                            break;
                        }
                        var.push(inner);
                    }
                    if !var.is_empty() {
                        vars.insert(var);
                    }
                }
            }
            vars
        } else {
            std::collections::HashSet::new()
        };

    // Build the param list: use `_name` unless this is a template action AND the
    // name appears in the template (in which case it must be addressable by name).
    let params_str: String = raw_params
        .iter()
        .map(|(name, ty)| {
            if is_template && template_vars.contains(*name) {
                format!("{name}: {ty}")
            } else {
                format!("_{name}: {ty}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let _ = writeln!(
        out,
        "        fn {method_name}(&mut self, {params_str}) -> VisitResult {{"
    );

    match action {
        crate::e2e::fixture::CallbackAction::Skip => {
            let _ = writeln!(out, "            VisitResult::Skip");
        }
        crate::e2e::fixture::CallbackAction::Continue => {
            let _ = writeln!(out, "            VisitResult::Continue");
        }
        crate::e2e::fixture::CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "            VisitResult::PreserveHtml");
        }
        crate::e2e::fixture::CallbackAction::Custom { output } => {
            let escaped = crate::e2e::escape::escape_rust(output);
            let _ = writeln!(out, "            VisitResult::Custom(\"{escaped}\".to_string())");
        }
        crate::e2e::fixture::CallbackAction::CustomTemplate { template, .. } => {
            // For any template-referenced param that is `Option<T>`, emit a shadow
            // `let name = name.unwrap_or_default();` so the format! string can use it.
            for (name, ty) in raw_params {
                if template_vars.contains(*name) && ty.starts_with("Option<") {
                    let _ = writeln!(out, "            let {name} = {name}.unwrap_or_default();");
                }
            }
            let escaped = crate::e2e::escape::escape_rust(template);
            let _ = writeln!(out, "            VisitResult::Custom(format!(\"{escaped}\"))");
        }
    }
    let _ = writeln!(out, "        }}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_to_rust_literal_null_returns_none() {
        let out = json_to_rust_literal(&serde_json::Value::Null, "string");
        assert_eq!(out, "None");
    }

    #[test]
    fn resolve_visitor_trait_uses_config() {
        use crate::e2e::config::CallOverride;

        let override_with_trait = CallOverride {
            visitor_trait: Some("HtmlVisitor".to_string()),
            ..Default::default()
        };
        assert_eq!(
            resolve_visitor_trait(Some(&override_with_trait)),
            Some("HtmlVisitor".to_string())
        );

        let override_without_trait = CallOverride::default();
        assert_eq!(resolve_visitor_trait(Some(&override_without_trait)), None);

        assert_eq!(resolve_visitor_trait(None), None);
    }

    #[test]
    fn handle_arg_deserializes_with_generic_config_type() {
        let (lines, expr) = render_rust_arg(
            "session",
            &serde_json::json!({"limit": 3}),
            "handle",
            false,
            "sample",
            "fixture",
            None,
            false,
            None,
            "test_documents",
            false,
            Some("SessionConfig"),
            &[],
            false,
            false,
        );

        assert_eq!(expr, "&session");
        let rendered = lines.join("\n");
        assert!(rendered.contains("let session_config: SessionConfig = serde_json::from_str"));
        assert!(rendered.contains("create_session(Some(session_config))"));
        assert!(!rendered.contains("CrawlConfig"));
    }

    #[test]
    fn json_object_with_vec_inner_is_ref_emits_str_shim() {
        let (lines, expr) = render_rust_arg(
            "names",
            &serde_json::json!(["python", "rust"]),
            "json_object",
            false,
            "my_crate",
            "fixture_id",
            None,
            false,
            Some("String"),
            "test_documents",
            false,
            None,
            &[],
            true,
            false,
        );

        assert_eq!(expr, "&names_refs");
        let rendered = lines.join("\n");
        assert!(rendered.contains("let names_refs: Vec<&str> = names.iter().map(String::as_str).collect();"));
    }

    #[test]
    fn json_object_without_vec_inner_is_ref_emits_plain_ref() {
        let (lines, expr) = render_rust_arg(
            "names",
            &serde_json::json!(["python", "rust"]),
            "json_object",
            false,
            "my_crate",
            "fixture_id",
            None,
            false,
            Some("String"),
            "test_documents",
            false,
            None,
            &[],
            false,
            false,
        );

        assert_eq!(expr, "&names");
        let rendered = lines.join("\n");
        assert!(!rendered.contains("names_refs"));
    }

    #[test]
    fn json_object_element_type_for_object_emits_single_dto() {
        let (lines, expr) = render_rust_arg(
            "input",
            &serde_json::json!({"kind": "uri", "uri": "sample.txt"}),
            "json_object",
            false,
            "my_crate",
            "fixture_id",
            None,
            true,
            Some("ExtractInput"),
            "test_documents",
            false,
            None,
            &[],
            false,
            false,
        );

        assert_eq!(expr, "input");
        let rendered = lines.join("\n");
        assert!(rendered.contains("serde_json::from_value::<ExtractInput>(input_json).unwrap()"));
        assert!(!rendered.contains("Vec<ExtractInput>"));
    }

    #[test]
    fn json_object_mock_url_placeholder_uses_mock_server_base() {
        let (lines, expr) = render_rust_arg(
            "input",
            &serde_json::json!({"kind": "uri", "uri": "$mock_url/document.txt"}),
            "json_object",
            false,
            "my_crate",
            "url_fixture",
            None,
            true,
            Some("ExtractInput"),
            "test_documents",
            false,
            None,
            &[],
            false,
            false,
        );

        assert_eq!(expr, "input");
        let rendered = lines.join("\n");
        assert!(rendered.contains("MOCK_SERVER_URL_FIXTURE"));
        assert!(rendered.contains(".replace(\"$mock_url\", &input_mock_base_url)"));
    }

    #[test]
    fn docs_bytes_read_the_presented_relative_path() {
        let (lines, expression) = render_rust_arg(
            "content",
            &serde_json::json!("document.pdf"),
            "bytes",
            false,
            "sample",
            "fixture",
            None,
            true,
            None,
            "test_documents",
            false,
            None,
            &[FixtureDocsFileInput {
                field: String::new(),
                path: "document.pdf".into(),
            }],
            false,
            false,
        );

        assert_eq!(expression, "content");
        assert_eq!(
            lines,
            vec![r##"let content = std::fs::read(r#"document.pdf"#).expect("file read failed");"##]
        );
    }

    #[test]
    fn docs_nested_bytes_replace_the_typed_json_field() {
        let (lines, expression) = render_rust_arg(
            "input",
            &serde_json::json!({"kind": "bytes", "bytes": "document.pdf"}),
            "json_object",
            false,
            "sample",
            "fixture",
            None,
            true,
            Some("DocumentInput"),
            "test_documents",
            false,
            None,
            &[FixtureDocsFileInput {
                field: "/bytes".into(),
                path: "document.pdf".into(),
            }],
            false,
            false,
        );

        assert_eq!(expression, "input");
        let rendered = lines.join("\n");
        assert!(rendered.contains(r##"std::fs::read(r#"document.pdf"#)"##), "{rendered}");
        assert!(
            rendered.contains(r##"input_json.pointer_mut(r#"/bytes"#)"##),
            "{rendered}"
        );
        assert!(rendered.contains("let mut input_json:"), "{rendered}");
        assert!(
            rendered.contains("serde_json::from_value::<DocumentInput>"),
            "{rendered}"
        );
    }

    #[test]
    fn docs_nested_bytes_generated_rust_compiles() {
        // This spawns a real `cargo check`, which takes cargo's machine-wide package-cache
        // lock. `REAL_CARGO_LOCK`'s doc records that lock actually being contended during
        // this crate's own suite ("Blocking waiting for file lock on package cache"), so
        // every in-process test that launches real cargo has to serialize against the rest.
        // `--offline` narrows the window but does not remove it: the index read still locks.
        // Cross-process contention (a concurrent `alef` run in another worktree) is outside
        // what a process-local mutex can reach -- this closes the in-process half only. ~keep
        let _cargo_lock = crate::test_support::RealCargoGuard::acquire();
        let (lines, expression) = render_rust_arg(
            "input",
            &serde_json::json!({"bytes": "document.pdf"}),
            "json_object",
            false,
            "sample",
            "fixture",
            None,
            true,
            Some("DocumentInput"),
            "test_documents",
            false,
            None,
            &[FixtureDocsFileInput {
                field: "/bytes".into(),
                path: "document.pdf".into(),
            }],
            false,
            false,
        );
        let directory = tempfile::tempdir().expect("temporary directory");
        std::fs::create_dir(directory.path().join("src")).expect("create source directory");
        std::fs::write(
            directory.path().join("Cargo.toml"),
            "[package]\nname = \"generated-check\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\n",
        )
        .expect("write manifest");
        let body = lines.join("\n    ");
        // `#[allow(dead_code)]`: this harness only ever constructs `DocumentInput` to prove
        // the emitted deserialization expression type-checks -- nothing reads `.bytes`
        // afterward. `#[derive(Deserialize)]` writes the field but rustc's dead-code
        // analysis only tracks reads, so without the allow this fails under `-D warnings`
        // (which CI's `RUSTFLAGS` sets ambiently and this spawned `cargo check` inherits),
        // even though the snippet is valid Rust. ~keep
        let source = format!(
            "#[derive(serde::Deserialize)]\n#[allow(dead_code)]\nstruct DocumentInput {{ bytes: Vec<u8> }}\nfn main() {{\n    {body}\n    let _input: DocumentInput = {expression};\n}}\n"
        );
        std::fs::write(directory.path().join("src/main.rs"), source).expect("write Rust source");
        std::fs::write(directory.path().join("document.pdf"), b"document").expect("write fixture file");
        let output = std::process::Command::new("cargo")
            .args(["check", "--offline", "--quiet"])
            .env("CARGO_TARGET_DIR", directory.path().join("target"))
            .current_dir(directory.path())
            .output()
            .expect("run Rust compiler");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    /// `extract_batch`-shaped fixtures pass an array (`Vec<ExtractInput>`, `element_type =
    /// Some("ExtractInput")`) whose docs file pointer targets one element of that array
    /// (e.g. `/1/bytes`, mirroring `fixtures/batch/bytes_happy.json`). The `_json` binding is
    /// mutated in place via `pointer_mut` before being deserialized, so it must be declared
    /// `let mut`, exactly like the single-object case covered by
    /// `docs_nested_bytes_replace_the_typed_json_field` above.
    #[test]
    fn should_emit_mut_binding_when_batch_array_element_is_mutably_borrowed_via_docs_file() {
        let (lines, expression) = render_rust_arg(
            "inputs",
            &serde_json::json!([
                {"kind": "bytes", "bytes": [1, 2, 3], "mime_type": "text/plain"},
                {"kind": "bytes", "bytes": "placeholder", "mime_type": "text/html"}
            ]),
            "json_object",
            false,
            "sample",
            "fixture",
            None,
            true,
            Some("ExtractInput"),
            "test_documents",
            false,
            None,
            &[FixtureDocsFileInput {
                field: "/1/bytes".into(),
                path: "html/html.html".into(),
            }],
            false,
            false,
        );

        assert_eq!(expression, "inputs");
        let rendered = lines.join("\n");
        assert!(rendered.contains("let mut inputs_json:"), "{rendered}");
        assert!(
            rendered.contains(r##"inputs_json.pointer_mut(r#"/1/bytes"#)"##),
            "{rendered}"
        );
        assert!(
            rendered.contains("serde_json::from_value::<Vec<ExtractInput>>(inputs_json).unwrap()"),
            "{rendered}"
        );
    }

    /// Sibling control for the test above: a `json_object` arg with no docs file replacement
    /// never calls `pointer_mut`, so the binding must stay immutable. This guards against a
    /// blanket `let mut` fix that would trip Rust's `unused_mut` lint on every other
    /// `json_object` argument in the generated e2e suite.
    #[test]
    fn should_not_emit_mut_binding_when_json_object_arg_has_no_docs_file() {
        let (lines, expression) = render_rust_arg(
            "inputs",
            &serde_json::json!([
                {"kind": "bytes", "bytes": [1, 2, 3], "mime_type": "text/plain"}
            ]),
            "json_object",
            false,
            "sample",
            "fixture",
            None,
            true,
            Some("ExtractInput"),
            "test_documents",
            false,
            None,
            &[],
            false,
            false,
        );

        assert_eq!(expression, "inputs");
        let rendered = lines.join("\n");
        assert!(rendered.contains("let inputs_json:"), "{rendered}");
        assert!(!rendered.contains("mut"), "{rendered}");
        assert!(!rendered.contains("pointer_mut"), "{rendered}");
    }
}
