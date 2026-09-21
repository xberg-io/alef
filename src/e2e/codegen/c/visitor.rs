//! C e2e visitor fixture test generation.

use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{escape_c, sanitize_ident};
use crate::e2e::fixture::Fixture;
use heck::{ToPascalCase, ToSnakeCase};
use std::fmt::Write as FmtWrite;

use super::{CallIr, json_to_c, resolve_call_info};

// ---------------------------------------------------------------------------
// Visitor test file generation for C FFI
// ---------------------------------------------------------------------------

/// Generate `test_visitor.c` — one test function per visitor-bearing fixture.
///
/// Each test:
/// 1. Defines static C callback functions for each configured callback slot.
/// 2. Zero-initialises the generated visitor callback struct and wires each slot.
/// 3. Creates a visitor handle via the configured FFI prefix.
/// 4. Creates an options handle via the resolved options type's `from_json` symbol.
/// 5. Attaches the visitor via the configured FFI prefix.
/// 6. Calls the configured C FFI function and serialises the result to JSON.
/// 7. Extracts fields via `alef_json_get_string` and runs `contains`/`not_contains`
///    assertions with `assert(…)`.
/// 8. Frees all handles in reverse allocation order.
pub(super) fn render_visitor_test_file(
    fixtures: &[&Fixture],
    header: &str,
    _prefix: &str,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    ir: CallIr<'_>,
) -> anyhow::Result<String> {
    use crate::e2e::fixture::CallbackAction;

    let mut out = String::new();
    out.push_str(&hash::e2e_header(CommentStyle::Block));
    let _ = writeln!(out, "/* E2e tests for category: visitor */");
    let _ = writeln!(out);
    let _ = writeln!(out, "#include <assert.h>");
    let _ = writeln!(out, "#include <stdint.h>");
    let _ = writeln!(out, "#include <string.h>");
    let _ = writeln!(out, "#include <stdio.h>");
    let _ = writeln!(out, "#include <stdlib.h>");
    let mut headers = std::collections::BTreeSet::from([header.to_string()]);
    for fixture in fixtures {
        let call = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        headers.insert(super::effective_c_header(call, config));
    }
    for header in headers {
        let _ = writeln!(out, "#include \"{header}\"");
    }
    let _ = writeln!(out, "#include \"test_runner.h\"");
    let _ = writeln!(out);

    // Header *type* names carry cbindgen's `[export] prefix`, which is shouty-snake, not a bare
    // uppercase: the two disagree for any prefix with an internal word boundary. Derive it from
    // the same helper the header producer uses so the emitted code cannot name a type the header
    // never declares. ~keep
    for (i, fixture) in fixtures.iter().enumerate() {
        let fn_name = sanitize_ident(&fixture.id);
        let description = &fixture.description;
        let call_config = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        // `None`: visitor fixtures are the vtable/callback-shaped trait-bridge pattern, never
        // the register_fn/unregister_fn/clear_fn registry-operation shape
        // `trait_bridge_derived_c_identity` classifies, so there is nothing for it to match
        // here. ~keep
        let call_info = resolve_call_info(call_config, "c", ir, None);
        let prefix = super::effective_c_prefix(call_config, config);
        let prefix_upper = crate::codegen::c_consumer::export_type_prefix(&prefix);
        let visitor_type_stem = prefix.to_pascal_case();
        let visitor_callbacks_type = format!("{prefix_upper}{visitor_type_stem}VisitorCallbacks");
        let visitor_context_type = format!("{prefix_upper}{visitor_type_stem}Context");
        let function_name = call_info.function_name.as_str();
        let options_type_name = call_info.options_type_name.as_str();
        let options_type_snake = options_type_name.to_snake_case();
        let visitor_spec = match &fixture.visitor {
            Some(v) => v,
            None => continue,
        };

        // After the `continue` above, not before it: a fixture this emitter skips outright must
        // not be able to fail generation. For the ones it does emit, the name is spelled into two
        // symbols nothing else checks — `{prefix}_{result_snake}_to_json` and
        // `{prefix}_{result_snake}_free` — so an unresolvable result type has to stop generation
        // rather than name a pair of exports the header never declares. ~keep
        let result_type_name = call_info.result_type_name.require()?;
        let result_type_snake = result_type_name.to_snake_case();

        let html = fixture.input.get("html").and_then(|v| v.as_str()).unwrap_or("");
        let html_escaped = escape_c(html);

        let options_json = match fixture.input.get("options") {
            Some(opts) => serde_json::to_string(opts).unwrap_or_else(|_| "{}".to_string()),
            None => "{}".to_string(),
        };
        let options_escaped = escape_c(&options_json);

        // Emit static callback functions for this fixture. Each callback is named
        // `c_visitor_<fixture_id>_<method>` to avoid collisions across fixtures.
        let mut sorted_callbacks: Vec<(&String, &CallbackAction)> = visitor_spec.callbacks.iter().collect();
        sorted_callbacks.sort_by(|a, b| a.0.cmp(b.0));

        for (method, action) in &sorted_callbacks {
            let cb_name = format!("c_visitor_{fn_name}_{method}");
            let params = c_visitor_callback_params(method, &visitor_context_type);
            let body = c_visitor_callback_body(method, action);
            let _ = writeln!(out, "static int32_t {cb_name}({params}) {{");
            out.push_str(&body);
            let _ = writeln!(out, "}}");
            let _ = writeln!(out);
        }

        // Emit the test function.
        let _ = writeln!(out, "void test_{fn_name}(void) {{");
        let _ = writeln!(out, "    /* {description} */");
        let _ = writeln!(out);

        // Build callbacks struct and wire each slot.
        let _ = writeln!(out, "    {visitor_callbacks_type} _callbacks;");
        let _ = writeln!(out, "    memset(&_callbacks, 0, sizeof(_callbacks));");
        for (method, _) in &sorted_callbacks {
            let cb_name = format!("c_visitor_{fn_name}_{method}");
            let _ = writeln!(out, "    _callbacks.{method} = {cb_name};");
        }
        let _ = writeln!(out);

        // Create visitor handle.
        out.push_str(&crate::e2e::template_env::render(
            "c/managed_handle_create.jinja",
            minijinja::context! {
                prefix_upper => &prefix_upper,
                handle => "_visitor",
                expression => format!("{prefix}_visitor_create(&_callbacks)"),
                failure_message => "visitor create failed",
            },
        ));
        let _ = writeln!(out);

        // Create options handle.
        let _ = writeln!(
            out,
            "    {prefix_upper}AlefHandle _options = {prefix}_{options_type_snake}_from_json(\"{options_escaped}\");"
        );
        let _ = writeln!(out, "    assert(_options != 0 && \"options from_json failed\");");
        let _ = writeln!(out);

        // Attach visitor to options.
        let _ = writeln!(out, "    {prefix}_options_set_visitor(_options, _visitor);");
        let _ = writeln!(out);

        // Call the configured C FFI function.
        let _ = writeln!(
            out,
            "    {prefix_upper}AlefHandle _result = {function_name}(\"{html_escaped}\", _options);"
        );
        let _ = writeln!(out, "    assert(_result != 0 && \"visitor call failed\");");
        let _ = writeln!(out);

        if !fixture.assertions.is_empty() {
            let _ = writeln!(out, "    char* _json = {prefix}_{result_type_snake}_to_json(_result);");
            let _ = writeln!(out, "    assert(_json != NULL && \"result to_json failed\");");
            let _ = writeln!(out, "    char* _content = alef_json_get_string(_json, \"content\");");
            let _ = writeln!(out);
        }

        // Emit assertions (only contains/not_contains; visitor fixtures use only these).
        for assertion in &fixture.assertions {
            match assertion.assertion_type.as_str() {
                "contains" => {
                    if let Some(expected) = &assertion.value {
                        let c_val = json_to_c(expected);
                        let _ = writeln!(
                            out,
                            "    assert(_content != NULL && strstr(_content, {c_val}) != NULL && \"expected to contain substring\");"
                        );
                    }
                }
                "not_contains" => {
                    if let Some(expected) = &assertion.value {
                        let c_val = json_to_c(expected);
                        let _ = writeln!(
                            out,
                            "    assert((_content == NULL || strstr(_content, {c_val}) == NULL) && \"expected NOT to contain substring\");"
                        );
                    }
                }
                other => {
                    let _ = writeln!(
                        out,
                        "    /* assertion type '{other}' not supported in C visitor tests */"
                    );
                }
            }
        }

        let _ = writeln!(out);

        // Free in reverse allocation order.
        if !fixture.assertions.is_empty() {
            let _ = writeln!(out, "    free(_content);");
            let _ = writeln!(out, "    {prefix}_free_string(_json);");
        }
        let _ = writeln!(out, "    {prefix}_{result_type_snake}_free(_result);");
        let _ = writeln!(out, "    {prefix}_{options_type_snake}_free(_options);");
        let _ = writeln!(out, "    {prefix}_visitor_free(_visitor);");
        let _ = writeln!(out, "}}");

        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    Ok(out)
}

pub(super) fn render_visitor_snippet(
    fixture: &Fixture,
    header: &str,
    prefix: &str,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    ir: CallIr<'_>,
) -> anyhow::Result<String> {
    let mut snippet_fixture = fixture.clone();
    snippet_fixture.assertions.clear();
    let rendered = render_visitor_test_file(&[&snippet_fixture], header, prefix, e2e_config, config, ir)?;
    let function_marker = format!("void test_{}(void) {{", sanitize_ident(&fixture.id));
    let function_start = rendered
        .find(&function_marker)
        .ok_or_else(|| anyhow::anyhow!("C visitor snippet `{}` did not emit a test function", fixture.id))?;
    let declarations_start = rendered[..function_start].find("static ").unwrap_or(function_start);
    let declarations = rendered[declarations_start..function_start].trim_end();
    let body_start = function_start + function_marker.len();
    let body_end = rendered[body_start..]
        .rfind("\n}")
        .map(|offset| body_start + offset)
        .ok_or_else(|| anyhow::anyhow!("C visitor snippet `{}` emitted an unterminated function", fixture.id))?;
    let body = rendered[body_start..body_end]
        .lines()
        .map(|line| line.strip_prefix("    ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(crate::e2e::template_env::render(
        "c/snippet_body.jinja",
        minijinja::context! { header => header, declarations => declarations, body => body },
    ))
}

/// C function-pointer parameter list for a given visitor callback method.
///
/// Mirrors the cbindgen-emitted visitor callback slot signatures from
/// the generated FFI header. Named parameters
/// are prefixed with `_` so the C compiler does not warn about unused params when
/// the callback body ignores them.
fn c_visitor_callback_params(method: &str, context_type: &str) -> String {
    match method {
        "visit_text" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* _text, char** out_custom, size_t* out_len"
            )
        }
        "visit_element_start" => {
            format!("const {context_type}* _ctx, void* _user_data, char** out_custom, size_t* out_len")
        }
        "visit_element_end" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* _output, char** out_custom, size_t* out_len"
            )
        }
        "visit_link" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* _href, const char* _text, const char* _title, char** out_custom, size_t* out_len"
            )
        }
        "visit_image" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* _src, const char* _alt, const char* _title, char** out_custom, size_t* out_len"
            )
        }
        "visit_heading" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, uint32_t _level, const char* _text, const char* _id, char** out_custom, size_t* out_len"
            )
        }
        "visit_code_block" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* _lang, const char* _code, char** out_custom, size_t* out_len"
            )
        }
        "visit_code_inline" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* _code, char** out_custom, size_t* out_len"
            )
        }
        "visit_list_item" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, int32_t _ordered, const char* _marker, const char* _text, char** out_custom, size_t* out_len"
            )
        }
        "visit_list_start" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, int32_t _ordered, char** out_custom, size_t* out_len"
            )
        }
        "visit_list_end" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, int32_t _ordered, const char* _output, char** out_custom, size_t* out_len"
            )
        }
        "visit_table_start" => {
            format!("const {context_type}* _ctx, void* _user_data, char** out_custom, size_t* out_len")
        }
        "visit_table_row" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* const* _cells, size_t _cell_count, int32_t _is_header, char** out_custom, size_t* out_len"
            )
        }
        "visit_table_end" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* _output, char** out_custom, size_t* out_len"
            )
        }
        "visit_blockquote" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* _content, size_t _depth, char** out_custom, size_t* out_len"
            )
        }
        "visit_line_break" | "visit_horizontal_rule" | "visit_definition_list_start" | "visit_figure_start" => {
            format!("const {context_type}* _ctx, void* _user_data, char** out_custom, size_t* out_len")
        }
        "visit_custom_element" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* _tag_name, const char* _html, char** out_custom, size_t* out_len"
            )
        }
        "visit_form" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* _action, const char* _method, char** out_custom, size_t* out_len"
            )
        }
        "visit_input" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* _input_type, const char* _name, const char* _value, char** out_custom, size_t* out_len"
            )
        }
        "visit_audio" | "visit_video" | "visit_iframe" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* _src, char** out_custom, size_t* out_len"
            )
        }
        "visit_details" => {
            format!("const {context_type}* _ctx, void* _user_data, int32_t _open, char** out_custom, size_t* out_len")
        }
        "visit_figure_end" | "visit_definition_list_end" => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* _output, char** out_custom, size_t* out_len"
            )
        }
        // Default: single text payload (covers visit_strong, visit_emphasis,
        // visit_strikethrough, visit_underline, visit_subscript, visit_superscript,
        // visit_mark, visit_button, visit_summary, visit_figcaption,
        // visit_definition_term, visit_definition_description).
        _ => {
            format!(
                "const {context_type}* _ctx, void* _user_data, const char* _text, char** out_custom, size_t* out_len"
            )
        }
    }
}

/// Build the body of a C visitor callback function for a given action.
///
/// Return values mirror the legacy visitor FFI discriminants:
///   0 = Continue, 1 = Skip, 2 = PreserveHtml, 3 = Custom.
///
/// For `Custom` and `CustomTemplate`, we heap-allocate a copy of the output string
/// with `strdup` (or a sprintf-allocated buffer) and pass its pointer and length back
/// via `out_custom`/`out_len`. The FFI runtime takes ownership and frees it.
fn c_visitor_callback_body(method: &str, action: &crate::e2e::fixture::CallbackAction) -> String {
    use crate::e2e::fixture::CallbackAction;

    let mut out = String::new();
    // Suppress unused-parameter warnings for context and user_data — always ignored
    // in simple e2e test callbacks.
    let _ = writeln!(out, "    (void)_ctx;");
    let _ = writeln!(out, "    (void)_user_data;");

    match action {
        CallbackAction::Skip => {
            let _ = writeln!(out, "    (void)out_custom;");
            let _ = writeln!(out, "    (void)out_len;");
            // Suppress method-specific params not used by Skip.
            for param in c_visitor_unused_params(method) {
                let _ = writeln!(out, "    (void){param};");
            }
            let _ = writeln!(out, "    return 2;");
        }
        CallbackAction::Continue => {
            let _ = writeln!(out, "    (void)out_custom;");
            let _ = writeln!(out, "    (void)out_len;");
            for param in c_visitor_unused_params(method) {
                let _ = writeln!(out, "    (void){param};");
            }
            let _ = writeln!(out, "    return 0;");
        }
        CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "    (void)out_custom;");
            let _ = writeln!(out, "    (void)out_len;");
            for param in c_visitor_unused_params(method) {
                let _ = writeln!(out, "    (void){param};");
            }
            let _ = writeln!(out, "    return 3;");
        }
        CallbackAction::Custom { output } => {
            let escaped = escape_c(output);
            for param in c_visitor_unused_params(method) {
                let _ = writeln!(out, "    (void){param};");
            }
            let _ = writeln!(out, "    char* _buf = strdup(\"{escaped}\");");
            let _ = writeln!(out, "    if (out_custom) *out_custom = _buf;");
            let _ = writeln!(out, "    if (out_len) *out_len = _buf ? strlen(_buf) : 0;");
            let _ = writeln!(out, "    return 1;");
        }
        CallbackAction::CustomTemplate { template, .. } => {
            // Build a sprintf format string and map fixture placeholders to C params.
            let (c_fmt, placeholders) = c_visitor_template_to_sprintf(template);
            let escaped_fmt = escape_c(&c_fmt);

            // Determine which method-specific params are used by the template.
            let used: std::collections::HashSet<&str> = placeholders.iter().map(|s| s.as_str()).collect();
            for param in c_visitor_unused_params(method) {
                let stripped = param.trim_start_matches('_');
                if !used.contains(stripped) {
                    let _ = writeln!(out, "    (void){param};");
                }
            }

            if placeholders.is_empty() {
                let _ = writeln!(out, "    char* _buf = strdup(\"{escaped_fmt}\");");
            } else {
                // Compute the max output length. We over-estimate by adding 256 per
                // placeholder plus the template length.
                let max_len = template.len() + placeholders.len() * 256 + 64;
                let _ = writeln!(out, "    char* _buf = (char*)malloc({max_len});");
                let _ = writeln!(out, "    if (!_buf) {{ (void)out_custom; (void)out_len; return 0; }}");
                // Build the sprintf argument list.
                let args: Vec<String> = placeholders
                    .iter()
                    .map(|name| c_visitor_placeholder_to_arg(method, name))
                    .collect();
                let args_str = args.join(", ");
                let _ = writeln!(out, "    snprintf(_buf, {max_len}, \"{escaped_fmt}\", {args_str});");
            }

            let _ = writeln!(out, "    if (out_custom) *out_custom = _buf;");
            let _ = writeln!(out, "    if (out_len) *out_len = _buf ? strlen(_buf) : 0;");
            let _ = writeln!(out, "    return 1;");
        }
    }

    out
}

/// List of method-specific typed C parameter names to suppress with `(void)` when
/// the callback body does not reference them.  Mirrors `unused_params_for` in
/// `zig_visitors.rs` but uses the C parameter names from `c_visitor_callback_params`.
fn c_visitor_unused_params(method: &str) -> Vec<&'static str> {
    match method {
        "visit_text" => vec!["_text"],
        "visit_element_start"
        | "visit_table_start"
        | "visit_line_break"
        | "visit_horizontal_rule"
        | "visit_definition_list_start"
        | "visit_figure_start" => vec![],
        "visit_element_end" | "visit_table_end" | "visit_figure_end" | "visit_definition_list_end" => {
            vec!["_output"]
        }
        "visit_link" => vec!["_href", "_text", "_title"],
        "visit_image" => vec!["_src", "_alt", "_title"],
        "visit_heading" => vec!["_level", "_text", "_id"],
        "visit_code_block" => vec!["_lang", "_code"],
        "visit_code_inline" => vec!["_code"],
        "visit_list_item" => vec!["_ordered", "_marker", "_text"],
        "visit_list_start" => vec!["_ordered"],
        "visit_list_end" => vec!["_ordered", "_output"],
        "visit_table_row" => vec!["_cells", "_cell_count", "_is_header"],
        "visit_blockquote" => vec!["_content", "_depth"],
        "visit_custom_element" => vec!["_tag_name", "_html"],
        "visit_form" => vec!["_action", "_method"],
        "visit_input" => vec!["_input_type", "_name", "_value"],
        "visit_audio" | "visit_video" | "visit_iframe" => vec!["_src"],
        "visit_details" => vec!["_open"],
        // Default: text-only methods.
        _ => vec!["_text"],
    }
}

/// Convert a fixture `{placeholder}` template into a `printf`/`snprintf` format string
/// and an ordered list of placeholder names.  Integer placeholders use `%d` or `%u`;
/// everything else uses `%s`.
fn c_visitor_template_to_sprintf(template: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(template.len());
    let mut placeholders: Vec<String> = Vec::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    out.push('{');
                    continue;
                }
                let mut name = String::new();
                while let Some(&peek) = chars.peek() {
                    if peek == '}' {
                        chars.next();
                        break;
                    }
                    name.push(peek);
                    chars.next();
                }
                let is_int = matches!(name.as_str(), "level" | "depth" | "ordered" | "open" | "is_header");
                if is_int {
                    out.push_str("%d");
                } else {
                    out.push_str("%s");
                }
                placeholders.push(name);
            }
            '}' => {
                if chars.peek() == Some(&'}') {
                    chars.next();
                }
                out.push('}');
            }
            '%' => {
                // Escape literal percent signs for printf.
                out.push_str("%%");
            }
            other => out.push(other),
        }
    }
    (out, placeholders)
}

/// Map a fixture placeholder name (e.g. `href`, `text`) to the C expression that
/// yields the value for that parameter slot in the callback's sprintf call.
fn c_visitor_placeholder_to_arg(method: &str, name: &str) -> String {
    let int_placeholder = matches!(
        (method, name),
        ("visit_heading", "level")
            | ("visit_blockquote", "depth")
            | ("visit_list_item", "ordered")
            | ("visit_list_start", "ordered")
            | ("visit_list_end", "ordered")
            | ("visit_details", "open")
            | ("visit_table_row", "is_header")
    );
    if int_placeholder {
        return format!("_{name}");
    }
    // String parameters — use the named `_<name>` C param directly.
    // The C param is already a `const char*`; pass it directly to `%s`.
    // Guard against NULL to avoid UB in printf (some implementations crash on NULL %s).
    format!("(_{name} ? _{name} : \"\")")
}

#[cfg(test)]
mod visitor_tests {
    use super::super::c_visitor_fixture_has_typed_call;
    use super::super::snippet_regressions::compile_snippet;
    use super::{CallIr, render_visitor_snippet, render_visitor_test_file};
    use crate::core::config::e2e::{CallConfig, CallOverride, E2eConfig};
    use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
    use crate::e2e::fixture::{Assertion, CallbackAction, Fixture, VisitorSpec};
    use std::collections::BTreeMap;

    fn visitor_fixture() -> Fixture {
        let mut callbacks = BTreeMap::new();
        callbacks.insert("visit_text".to_string(), CallbackAction::Continue);

        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "custom_names".to_string(),
            category: None,
            description: "uses configured names".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({
                "html": "<p>Hello</p>",
                "options": { "trim": true }
            }),
            mock_response: None,
            visitor: Some(VisitorSpec { callbacks }),
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![Assertion {
                skip: None,
                assertion_type: "contains".to_string(),
                field: None,
                value: Some(serde_json::json!("Hello")),
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: String::new(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }
    }

    fn e2e_config_with_c_call() -> E2eConfig {
        let c_override = CallOverride {
            function: Some("krz_render_document".to_string()),
            prefix: Some("krz".to_string()),
            options_type: Some("RenderConfig".to_string()),
            result_type: Some("RenderOutput".to_string()),
            ..Default::default()
        };
        let call = CallConfig {
            function: "render_document".to_string(),
            overrides: [("c".to_string(), c_override)].into(),
            ..Default::default()
        };
        E2eConfig {
            call,
            ..Default::default()
        }
    }

    fn crate_config_with_visitor_metadata() -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "Renderer".to_string(),
                context_type: Some("RenderContext".to_string()),
                result_type: Some("RenderDecision".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn c_visitor_file_uses_configured_call_types_and_symbols() {
        let fixture = visitor_fixture();
        let fixtures = vec![&fixture];
        let config = crate_config_with_visitor_metadata();
        let content = render_visitor_test_file(
            &fixtures,
            "krz.h",
            "krz",
            &e2e_config_with_c_call(),
            &config,
            CallIr::default(),
        )
        .expect("visitor test file renders");

        assert!(content.contains("KRZKrzVisitorCallbacks _callbacks"));
        assert!(content.contains("const KRZKrzContext* _ctx"));
        assert!(content.contains("KRZAlefHandle _options = krz_render_config_from_json"));
        assert!(content.contains("KRZAlefHandle _result = krz_render_document"));
        assert!(content.contains("char* _json = krz_render_output_to_json(_result);"));
        assert!(content.contains("krz_render_output_free(_result);"));
        assert!(content.contains("krz_render_config_free(_options);"));

        for hardcoded in [
            "DefaultOptions",
            "DefaultResult",
            "conversion_options_from_json",
            "conversion_result_to_json",
            "default_convert",
            "DEFDftVisitorCallbacks",
            "DEFDftSyntaxContext",
            "KRZKrzSyntaxContext",
        ] {
            assert!(
                !content.contains(hardcoded),
                "visitor C output leaked `{hardcoded}`:\n{content}"
            );
        }
    }

    #[test]
    fn c_visitor_snippet_reuses_callbacks_and_native_call_without_test_runner() {
        let fixture = visitor_fixture();
        let content = render_visitor_snippet(
            &fixture,
            "krz.h",
            "krz",
            &e2e_config_with_c_call(),
            &crate_config_with_visitor_metadata(),
            CallIr::default(),
        )
        .expect("visitor snippet renders");

        assert!(content.contains("static int32_t c_visitor_custom_names_visit_text"));
        assert!(content.contains("KRZAlefHandle _result = krz_render_document"));
        assert!(content.contains("int main(void)"));
        assert!(!content.contains("test_runner.h"));
        assert!(!content.contains("void test_custom_names"));
        assert!(!content.contains("alef_json_get_string"));
        assert!(!content.contains("krz_render_output_to_json"));
    }

    #[test]
    fn c_visitor_snippet_compiles_against_scalar_managed_handle_abi() {
        let fixture = visitor_fixture();
        let content = render_visitor_snippet(
            &fixture,
            "krz.h",
            "krz",
            &e2e_config_with_c_call(),
            &crate_config_with_visitor_metadata(),
            CallIr::default(),
        )
        .expect("visitor snippet renders");

        compile_snippet(
            &content,
            "krz.h",
            concat!(
                "#include <stddef.h>\n",
                "#include <stdint.h>\n",
                "typedef uint64_t KRZAlefHandle;\n",
                "typedef struct KRZKrzContext KRZKrzContext;\n",
                "typedef struct KRZKrzVisitor KRZKrzVisitor;\n",
                "typedef struct KRZKrzVisitorCallbacks {\n",
                "  int32_t (*visit_text)(const KRZKrzContext *, void *, const char *, char **, size_t *);\n",
                "} KRZKrzVisitorCallbacks;\n",
                "KRZAlefHandle krz_visitor_create(const KRZKrzVisitorCallbacks *callbacks);\n",
                "void krz_visitor_free(KRZAlefHandle visitor);\n",
                "KRZAlefHandle krz_render_config_from_json(const char *json);\n",
                "void krz_render_config_free(KRZAlefHandle options);\n",
                "void krz_options_set_visitor(KRZAlefHandle options, KRZAlefHandle visitor);\n",
                "KRZAlefHandle krz_render_document(const char *html, KRZAlefHandle options);\n",
                "void krz_render_output_free(KRZAlefHandle result);\n",
            ),
        );
    }

    #[test]
    fn c_visitor_fixture_without_typed_c_call_is_not_eligible() {
        let fixture = visitor_fixture();
        let config = E2eConfig::default();

        assert!(
            !c_visitor_fixture_has_typed_call(&fixture, &config, CallIr::default()),
            "visitor fixtures need a configured C function and options type"
        );
    }

    /// Net brace depth, ignoring braces inside string literals and comments — the emitted
    /// snippet carries both (`from_json("{}")`, `/* description */`).
    fn brace_depth(source: &str) -> i32 {
        let mut depth = 0;
        let mut in_string = false;
        let mut escaped = false;
        let mut chars = source.chars().peekable();
        while let Some(character) = chars.next() {
            if in_string {
                match character {
                    _ if escaped => escaped = false,
                    '\\' => escaped = true,
                    '"' => in_string = false,
                    _ => {}
                }
                continue;
            }
            match character {
                '"' => in_string = true,
                '/' if chars.peek() == Some(&'/') => {
                    for next in chars.by_ref() {
                        if next == '\n' {
                            break;
                        }
                    }
                }
                '/' if chars.peek() == Some(&'*') => {
                    let mut previous = ' ';
                    for next in chars.by_ref() {
                        if previous == '*' && next == '/' {
                            break;
                        }
                        previous = next;
                    }
                }
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
        depth
    }

    #[test]
    fn c_visitor_snippet_braces_balance() {
        let content = render_visitor_snippet(
            &visitor_fixture(),
            "krz.h",
            "krz",
            &e2e_config_with_c_call(),
            &crate_config_with_visitor_metadata(),
            CallIr::default(),
        )
        .expect("visitor snippet renders");

        assert_eq!(
            brace_depth(&content),
            0,
            "visitor snippet braces do not balance:\n{content}"
        );
    }

    /// The snippet must spell header *types* with cbindgen's `[export] prefix`, which is
    /// shouty-snake — not a bare uppercase of the symbol prefix. The two agree for every
    /// single-word prefix, so `SampleCore` is the only shape that can fail here: it is where
    /// `SAMPLE_CORE` and `SAMPLECORE` part ways.
    ///
    /// The header below is built from `c_consumer::export_type_prefix` / `handle_type` — the
    /// same helpers `gen_cbindgen_toml` uses — so the test compiles the snippet against the
    /// names the real header would carry rather than against a hand-copied spelling. A snippet
    /// that re-derives the prefix itself fails as `unknown type name`, which is exactly how it
    /// failed in a consumer repo. ~keep
    #[test]
    fn c_visitor_snippet_types_match_the_generated_header_export_prefix() {
        use crate::codegen::c_consumer::{export_type_prefix, handle_type};

        let prefix = "SampleCore";
        let export_prefix = export_type_prefix(prefix);
        let handle = handle_type(prefix);
        let c_override = CallOverride {
            function: Some(format!("{prefix}_render_document")),
            prefix: Some(prefix.to_string()),
            options_type: Some("RenderConfig".to_string()),
            result_type: Some("RenderOutput".to_string()),
            ..Default::default()
        };
        let e2e = E2eConfig {
            call: CallConfig {
                function: "render_document".to_string(),
                overrides: [("c".to_string(), c_override)].into(),
                ..Default::default()
            },
            ..Default::default()
        };

        let content = render_visitor_snippet(
            &visitor_fixture(),
            "sample_core.h",
            prefix,
            &e2e,
            &crate_config_with_visitor_metadata(),
            CallIr::default(),
        )
        .expect("visitor snippet renders");

        assert!(
            content.contains(&format!("{export_prefix}SampleCoreVisitorCallbacks")),
            "snippet must name the callbacks struct the header declares:\n{content}"
        );
        assert!(
            content.contains(&format!("{handle} _visitor =")),
            "the visitor is a scalar `{handle}`, never a pointer:\n{content}"
        );
        assert!(
            !content.contains("SAMPLECORE"),
            "snippet used a bare-uppercase prefix the header never declares:\n{content}"
        );

        // Substituted rather than `format!`ed so the C braces below stay readable and the two
        // placeholders can only ever be filled from the derived names above. ~keep
        const HEADER_TEMPLATE: &str = concat!(
            "#include <stddef.h>\n",
            "#include <stdint.h>\n",
            "typedef uint64_t <HANDLE>;\n",
            "typedef struct <PREFIX>SampleCoreContext <PREFIX>SampleCoreContext;\n",
            "typedef struct <PREFIX>SampleCoreVisitorCallbacks {\n",
            "  int32_t (*visit_text)(const <PREFIX>SampleCoreContext *, void *, const char *, char **, size_t *);\n",
            "} <PREFIX>SampleCoreVisitorCallbacks;\n",
            "<HANDLE> SampleCore_visitor_create(const <PREFIX>SampleCoreVisitorCallbacks *callbacks);\n",
            "void SampleCore_visitor_free(<HANDLE> visitor);\n",
            "<HANDLE> SampleCore_render_config_from_json(const char *json);\n",
            "void SampleCore_render_config_free(<HANDLE> options);\n",
            "void SampleCore_options_set_visitor(<HANDLE> options, <HANDLE> visitor);\n",
            "<HANDLE> SampleCore_render_document(const char *html, <HANDLE> options);\n",
            "void SampleCore_render_output_free(<HANDLE> result);\n",
        );
        let header = HEADER_TEMPLATE
            .replace("<HANDLE>", &handle)
            .replace("<PREFIX>", &export_prefix);

        compile_snippet(&content, "sample_core.h", &header);
    }
}
