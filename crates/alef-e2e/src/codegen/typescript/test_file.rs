//! Test file and test case rendering for TypeScript e2e tests.

use std::fmt::Write as FmtWrite;

use crate::config::{ArgMapping, E2eConfig};
use crate::escape::{escape_js, expand_fixture_templates, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::Fixture;
use alef_core::hash::{self, CommentStyle};
use heck::ToUpperCamelCase;

use super::assertions::render_assertion;
use super::json::{json_to_js, json_to_js_camel, snake_to_camel};
use super::visitors::build_typescript_visitor;

/// Render a complete test file for the given category.
///
/// `lang` is the language key used for per-fixture call override resolution
/// (e.g. `"node"` for TypeScript, `"wasm"` for WASM tests).
#[allow(clippy::too_many_arguments)]
pub fn render_test_file(
    lang: &str,
    category: &str,
    fixtures: &[&Fixture],
    module_path: &str,
    pkg_name: &str,
    function_name: &str,
    args: &[ArgMapping],
    options_type: Option<&str>,
    field_resolver: &FieldResolver,
    client_factory: Option<&str>,
    e2e_config: &E2eConfig,
) -> String {
    // `lang` is used for wasm visitor arg placement and override routing
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let _ = writeln!(out, "import {{ describe, expect, it }} from 'vitest';");

    let has_non_http_fixtures = fixtures.iter().any(|f| !f.is_http_test() && !f.assertions.is_empty());

    let needs_options_import = options_type.is_some()
        && fixtures.iter().any(|f| {
            args.iter().any(|arg| {
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                let val = if field == "input" {
                    Some(&f.input)
                } else {
                    f.input.get(field)
                };
                arg.arg_type == "json_object" && val.is_some_and(|v| !v.is_null())
            })
        });

    // Collect handle constructor function names that need to be imported.
    let handle_constructors: Vec<String> = args
        .iter()
        .filter(|arg| arg.arg_type == "handle")
        .map(|arg| format!("create{}", arg.name.to_upper_camel_case()))
        .collect();

    if has_non_http_fixtures {
        let mut imports: Vec<String> = if let Some(factory) = client_factory {
            vec![factory.to_string()]
        } else {
            vec![function_name.to_string()]
        };

        // Also import any additional function names used by per-fixture call overrides.
        for fixture in fixtures.iter().filter(|f| !f.is_http_test()) {
            if fixture.call.is_some() {
                let call_config = e2e_config.resolve_call(fixture.call.as_deref());
                let fixture_fn = resolve_node_function_name(call_config);
                if client_factory.is_none() && !imports.contains(&fixture_fn) {
                    imports.push(fixture_fn);
                }
            }
        }

        // Collect tree helper function names needed by method_result assertions.
        for fixture in fixtures.iter().filter(|f| !f.is_http_test()) {
            for assertion in &fixture.assertions {
                if assertion.assertion_type == "method_result" {
                    if let Some(method_name) = &assertion.method {
                        if let Some(helper_fn) = ts_method_helper_import(method_name) {
                            if !imports.contains(&helper_fn) {
                                imports.push(helper_fn);
                            }
                        }
                    }
                }
            }
        }

        for ctor in &handle_constructors {
            if !imports.contains(ctor) {
                imports.push(ctor.clone());
            }
        }

        // Detect batch item types (BatchBytesItem, BatchFileItem) used in any fixture
        for fixture in fixtures.iter() {
            let cc = e2e_config.resolve_call(fixture.call.as_deref());
            for arg in &cc.args {
                if let Some(elem_type) = &arg.element_type {
                    if (elem_type == "BatchBytesItem" || elem_type == "BatchFileItem") && !imports.contains(elem_type) {
                        imports.push(elem_type.clone());
                    }
                }
            }
        }

        let _ = module_path; // retained in signature for potential future use
        if let (true, Some(opts_type)) = (needs_options_import, options_type) {
            imports.push(format!("type {opts_type}"));
            let imports_str = imports.join(", ");
            let _ = writeln!(out, "import {{ {imports_str} }} from '{pkg_name}';");
        } else {
            let imports_str = imports.join(", ");
            let _ = writeln!(out, "import {{ {imports_str} }} from '{pkg_name}';");
        }
    }

    if has_non_http_fixtures {
        let _ = writeln!(out);
        let _ = writeln!(out, "function _alefE2eText(value: unknown): string {{");
        let _ = writeln!(out, "  return value == null ? \"\" : String(value);");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
        let _ = writeln!(out, "function _alefE2eItemTexts(item: unknown): string[] {{");
        let _ = writeln!(out, "  if (item == null || typeof item !== \"object\") {{");
        let _ = writeln!(out, "    return [_alefE2eText(item)];");
        let _ = writeln!(out, "  }}");
        let _ = writeln!(out, "  const record = item as Record<string, unknown>;");
        let _ = writeln!(
            out,
            "  const itemsText = Array.isArray(record.items) ? record.items.map(_alefE2eText).join(\" \") : \"\";"
        );
        let _ = writeln!(
            out,
            "  return [_alefE2eText(item), _alefE2eText(record.kind), _alefE2eText(record.name), _alefE2eText(record.source), _alefE2eText(record.alias), _alefE2eText(record.text), _alefE2eText(record.signature), itemsText];"
        );
        let _ = writeln!(out, "}}");
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "describe('{category}', () => {{");

    for (i, fixture) in fixtures.iter().enumerate() {
        if fixture.is_http_test() {
            render_http_test_case(&mut out, fixture);
        } else {
            render_test_case(
                &mut out,
                fixture,
                client_factory,
                options_type,
                field_resolver,
                e2e_config,
                lang,
            );
        }
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    let _ = writeln!(out, "}});");
    out
}

/// Resolve the function name for a call config, applying node-specific overrides.
pub(super) fn resolve_node_function_name(call_config: &crate::config::CallConfig) -> String {
    call_config
        .overrides
        .get("node")
        .and_then(|o| o.function.clone())
        .unwrap_or_else(|| snake_to_camel(&call_config.function))
}

/// Return the package-level helper function name to import for a method_result method,
/// or `None` if the method maps to a property access (no import needed).
pub(super) fn ts_method_helper_import(method_name: &str) -> Option<String> {
    match method_name {
        "has_error_nodes" => Some("treeHasErrorNodes".to_string()),
        "error_count" | "tree_error_count" => Some("treeErrorCount".to_string()),
        "tree_to_sexp" => Some("treeToSexp".to_string()),
        "contains_node_type" => Some("treeContainsNodeType".to_string()),
        "find_nodes_by_type" => Some("findNodesByType".to_string()),
        "run_query" => Some("runQuery".to_string()),
        _ => None,
    }
}

fn render_http_test_case(out: &mut String, fixture: &Fixture) {
    let Some(http) = &fixture.http else {
        return;
    };

    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.replace('\'', "\\'");

    if http.expected_response.status_code == 101 {
        let _ = writeln!(out, "  it.skip('{test_name}: {description}', async () => {{");
        let _ = writeln!(out, "    // HTTP 101 WebSocket upgrade cannot be tested via fetch");
        let _ = writeln!(out, "  }});");
        return;
    }

    let method = http.request.method.to_uppercase();
    let mut init_entries: Vec<String> = Vec::new();
    init_entries.push(format!("method: '{method}'"));
    init_entries.push("redirect: 'manual'".to_string());

    if !http.request.headers.is_empty() {
        let entries: Vec<String> = http
            .request
            .headers
            .iter()
            .map(|(k, v)| {
                let expanded_v = expand_fixture_templates(v);
                format!("      \"{}\": \"{}\"", escape_js(k), escape_js(&expanded_v))
            })
            .collect();
        init_entries.push(format!("headers: {{\n{},\n    }}", entries.join(",\n")));
    }

    if let Some(body) = &http.request.body {
        let js_body = json_to_js(body);
        init_entries.push(format!("body: JSON.stringify({js_body})"));
    }

    let fixture_id = escape_js(&fixture.id);
    let _ = writeln!(out, "  it('{test_name}: {description}', async () => {{");
    let _ = writeln!(
        out,
        "    const mockUrl = `${{process.env.MOCK_SERVER_URL}}/fixtures/{fixture_id}`;"
    );

    let init_str = init_entries.join(", ");
    let _ = writeln!(out, "    const response = await fetch(mockUrl, {{ {init_str} }});");

    let status = http.expected_response.status_code;
    let _ = writeln!(out, "    expect(response.status).toBe({status});");

    if let Some(expected_body) = &http.expected_response.body {
        if !(expected_body.is_null() || expected_body.is_string() && expected_body.as_str() == Some("")) {
            if let serde_json::Value::String(s) = expected_body {
                let escaped = escape_js(s);
                let _ = writeln!(out, "    const text = await response.text();");
                let _ = writeln!(out, "    expect(text).toBe('{escaped}');");
            } else {
                let js_val = json_to_js(expected_body);
                let _ = writeln!(out, "    const data = await response.json();");
                let _ = writeln!(out, "    expect(data).toEqual({js_val});");
            }
        }
    } else if let Some(partial) = &http.expected_response.body_partial {
        let _ = writeln!(out, "    const data = await response.json();");
        if let Some(obj) = partial.as_object() {
            for (key, val) in obj {
                let js_key = escape_js(key);
                let js_val = json_to_js(val);
                let _ = writeln!(
                    out,
                    "    expect((data as Record<string, unknown>)['{js_key}']).toEqual({js_val});"
                );
            }
        }
    }

    for (header_name, header_value) in &http.expected_response.headers {
        let lower_name = header_name.to_lowercase();
        if lower_name == "content-encoding" {
            continue;
        }
        let escaped_name = escape_js(&lower_name);
        match header_value.as_str() {
            "<<present>>" => {
                let _ = writeln!(
                    out,
                    "    expect(response.headers.get('{escaped_name}')).not.toBeNull();"
                );
            }
            "<<absent>>" => {
                let _ = writeln!(out, "    expect(response.headers.get('{escaped_name}')).toBeNull();");
            }
            "<<uuid>>" => {
                let _ = writeln!(
                    out,
                    "    expect(response.headers.get('{escaped_name}')).toMatch(/^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$/);"
                );
            }
            exact => {
                let escaped_val = escape_js(exact);
                let _ = writeln!(
                    out,
                    "    expect(response.headers.get('{escaped_name}')).toBe('{escaped_val}');"
                );
            }
        }
    }

    let body_has_content = matches!(&http.expected_response.body, Some(v)
        if !(v.is_null() || (v.is_string() && v.as_str() == Some(""))));
    if let Some(validation_errors) = &http.expected_response.validation_errors {
        if !validation_errors.is_empty() && !body_has_content {
            let _ = writeln!(
                out,
                "    const body = await response.json() as {{ errors?: unknown[] }};"
            );
            let _ = writeln!(out, "    const errors = body.errors ?? [];");
            for ve in validation_errors {
                let loc_js: Vec<String> = ve.loc.iter().map(|s| format!("\"{}\"", escape_js(s))).collect();
                let loc_str = loc_js.join(", ");
                let expanded_msg = expand_fixture_templates(&ve.msg);
                let escaped_msg = escape_js(&expanded_msg);
                let _ = writeln!(
                    out,
                    "    expect((errors as Array<Record<string, unknown>>).some((e) => JSON.stringify(e[\"loc\"]) === JSON.stringify([{loc_str}]) && String(e[\"msg\"]).includes(\"{escaped_msg}\"))).toBe(true);"
                );
            }
        }
    }

    let _ = writeln!(out, "  }});");
}

#[allow(clippy::too_many_arguments)]
fn render_test_case(
    out: &mut String,
    fixture: &Fixture,
    client_factory: Option<&str>,
    options_type: Option<&str>,
    field_resolver: &FieldResolver,
    e2e_config: &E2eConfig,
    lang: &str,
) {
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let function_name = resolve_node_function_name(call_config);
    let result_var = &call_config.result_var;
    let call_is_async = call_config.r#async;
    let args = &call_config.args;

    // Force test to async if we need to read files for bytes args
    let test_is_async = call_is_async || has_bytes_file_reads(&fixture.input, args);

    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.replace('\'', "\\'");
    let async_kw = if test_is_async { "async " } else { "" };
    let await_kw = if call_is_async { "await " } else { "" };

    let (mut setup_lines, args_str) = build_args_and_setup(&fixture.input, args, options_type, &fixture.id);

    let mut visitor_arg = String::new();
    if let Some(visitor_spec) = &fixture.visitor {
        visitor_arg = build_typescript_visitor(&mut setup_lines, visitor_spec);
    }

    let final_args = if visitor_arg.is_empty() {
        args_str
    } else if lang == "wasm" {
        // WASM: visitor must be merged into the options object (2nd arg), not appended as a
        // separate argument. The wasm binding accepts a single plain-object options param.
        if args_str.is_empty() {
            format!("{{ visitor: {visitor_arg} }}")
        } else if let Some(as_pos) = args_str.rfind(" as unknown as ") {
            let (before_cast, type_suffix) = args_str.split_at(as_pos);
            let merged_obj = if let Some(prefix) = before_cast.strip_suffix("{}") {
                format!("{prefix}{{ visitor: {visitor_arg} }}")
            } else if let Some(close_brace) = before_cast.rfind('}') {
                let (obj_body, _) = before_cast.split_at(close_brace);
                format!("{obj_body}, visitor: {visitor_arg} }}")
            } else {
                format!("{before_cast}, {{ visitor: {visitor_arg} }}")
            };
            format!("{merged_obj}{type_suffix}")
        } else {
            format!("{args_str}, {{ visitor: {visitor_arg} }}")
        }
    } else if args_str.is_empty() {
        format!("{{ visitor: {visitor_arg} }}")
    } else {
        format!("{args_str}, {{ visitor: {visitor_arg} }}")
    };

    let call_expr = if client_factory.is_some() {
        format!("client.{function_name}({final_args})")
    } else {
        format!("{function_name}({final_args})")
    };

    let base_url_expr = format!("`${{process.env.MOCK_SERVER_URL}}/fixtures/{}`", fixture.id);

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    if fixture.assertions.is_empty() {
        let _ = writeln!(out, "  it.skip('{test_name}: {description}', async () => {{");
        let _ = writeln!(out, "    // no assertions configured for this fixture in node e2e");
        let _ = writeln!(out, "  }});");
        return;
    }

    if expects_error {
        let _ = writeln!(out, "  it('{test_name}: {description}', async () => {{");
        if let Some(factory) = client_factory {
            let _ = writeln!(out, "    const client = {factory}('test-key', {base_url_expr});");
        }
        let _ = writeln!(out, "    await expect(async () => {{");
        for line in &setup_lines {
            let _ = writeln!(out, "      {line}");
        }
        let _ = writeln!(out, "      await {call_expr};");
        let _ = writeln!(out, "    }}).rejects.toThrow();");
        let _ = writeln!(out, "  }});");
        return;
    }

    let _ = writeln!(out, "  it('{test_name}: {description}', {async_kw}() => {{");

    if let Some(factory) = client_factory {
        let _ = writeln!(out, "    const client = {factory}('test-key', {base_url_expr});");
    }

    for line in &setup_lines {
        let _ = writeln!(out, "    {line}");
    }

    let has_usable_assertion = fixture.assertions.iter().any(|a| {
        if a.assertion_type == "not_error" || a.assertion_type == "error" {
            return false;
        }
        match &a.field {
            Some(f) if !f.is_empty() => field_resolver.is_valid_for_result(f),
            _ => true,
        }
    });

    if has_usable_assertion {
        let _ = writeln!(out, "    const {result_var} = {await_kw}{call_expr};");
    } else {
        let _ = writeln!(out, "    {await_kw}{call_expr};");
    }

    for assertion in &fixture.assertions {
        if assertion.assertion_type == "not_error" && !call_config.returns_result {
            continue;
        }
        render_assertion(out, assertion, result_var, field_resolver);
    }

    let _ = writeln!(out, "  }});");
}

/// Check whether any arg at index `idx` or later has a non-null value in `input`.
fn has_later_arg_value(args: &[ArgMapping], from_idx: usize, input: &serde_json::Value) -> bool {
    args[from_idx..].iter().any(|arg| {
        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = if field == "input" {
            Some(input)
        } else {
            input.get(field)
        };
        !matches!(val, None | Some(serde_json::Value::Null))
    })
}

/// Check if any arg with bytes type has a string path value that needs file reading.
fn has_bytes_file_reads(input: &serde_json::Value, args: &[ArgMapping]) -> bool {
    args.iter().any(|arg| {
        if arg.arg_type != "bytes" {
            return false;
        }
        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = if field == "input" {
            Some(input)
        } else {
            input.get(field)
        };
        matches!(val, Some(serde_json::Value::String(_)))
    })
}

/// Emit TypeScript batch item constructors for BatchBytesItem or BatchFileItem arrays.
fn emit_typescript_batch_item_array(arr: &serde_json::Value, elem_type: &str) -> String {
    if let Some(items) = arr.as_array() {
        let item_strs: Vec<String> = items
            .iter()
            .filter_map(|item| {
                if let Some(obj) = item.as_object() {
                    match elem_type {
                        "BatchBytesItem" => {
                            let content = obj.get("content").and_then(|v| v.as_array());
                            let mime_type = obj.get("mime_type").and_then(|v| v.as_str()).unwrap_or("text/plain");
                            let content_code = if let Some(arr) = content {
                                let bytes: Vec<String> =
                                    arr.iter().filter_map(|v| v.as_u64().map(|n| n.to_string())).collect();
                                format!("Buffer.from([{}])", bytes.join(", "))
                            } else {
                                "Buffer.from([])".to_string()
                            };
                            Some(format!("{{ content: {}, mimeType: \"{}\" }}", content_code, mime_type))
                        }
                        "BatchFileItem" => {
                            let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or("");
                            Some(format!("{{ path: \"{}\" }}", path.replace('\\', "\\\\")))
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .collect();
        format!("[{}]", item_strs.join(", "))
    } else {
        "[]".to_string()
    }
}

fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[ArgMapping],
    options_type: Option<&str>,
    fixture_id: &str,
) -> (Vec<String>, String) {
    if args.is_empty() {
        return (Vec::new(), json_to_js(input));
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for (idx, arg) in args.iter().enumerate() {
        if arg.arg_type == "mock_url" {
            setup_lines.push(format!(
                "const {} = `${{process.env.MOCK_SERVER_URL}}/fixtures/{fixture_id}`;",
                arg.name,
            ));
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "handle" {
            let constructor_name = format!("create{}", arg.name.to_upper_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("const {} = {constructor_name}(null);", arg.name));
            } else {
                let literal = json_to_js_camel(config_value);
                setup_lines.push(format!("const {name}Config = {literal};", name = arg.name));
                setup_lines.push(format!(
                    "const {} = {constructor_name}({name}Config);",
                    arg.name,
                    name = arg.name,
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = if field == "input" {
            Some(input)
        } else {
            input.get(field)
        };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                if has_later_arg_value(args, idx + 1, input) {
                    parts.push("undefined".to_string());
                }
            }
            None | Some(serde_json::Value::Null) => {
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                if arg.arg_type == "bytes" {
                    // For bytes type, if value is a string path, read the file
                    if let Some(path) = v.as_str() {
                        let var_name = format!("_{}_content", sanitize_ident(&arg.name));
                        setup_lines.push(format!(
                            "const {var_name} = await (await import('node:fs/promises')).readFile('{}');",
                            escape_js(path)
                        ));
                        parts.push(var_name);
                    } else {
                        // Binary array fallback
                        parts.push(format!("Buffer.from({})", json_to_js(v)));
                    }
                } else if arg.arg_type == "json_object" {
                    if v.is_array() {
                        // Array args (e.g. batch items) may need element_type wrapping.
                        if let Some(elem_type) = &arg.element_type {
                            if elem_type == "BatchBytesItem" || elem_type == "BatchFileItem" {
                                let wrapped = emit_typescript_batch_item_array(v, elem_type);
                                parts.push(wrapped);
                            } else {
                                parts.push(json_to_js_camel(v));
                            }
                        } else {
                            parts.push(json_to_js_camel(v));
                        }
                    } else if let Some(opts_type) = options_type {
                        // Object value with known options type — cast to the interface type.
                        if v.is_object() && v.as_object().is_some_and(|o| o.is_empty()) {
                            // Options types in TypeScript are interfaces, not classes — use object literal cast.
                            parts.push(format!("{{}} as {}", opts_type));
                        } else {
                            parts.push(format!("{} as {opts_type}", json_to_js_camel(v)));
                        }
                    } else {
                        parts.push(json_to_js_camel(v));
                    }
                    continue;
                } else {
                    parts.push(json_to_js(v));
                }
            }
        }
    }

    (setup_lines, parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::escape::sanitize_filename;
    use crate::fixture::FixtureGroup;

    #[test]
    fn resolve_node_function_name_converts_snake_to_camel() {
        use crate::config::CallConfig;
        let cc = CallConfig {
            function: "process_text".to_string(),
            ..Default::default()
        };
        assert_eq!(resolve_node_function_name(&cc), "processText");
    }

    #[test]
    fn ts_method_helper_import_recognizes_has_error_nodes() {
        assert_eq!(
            ts_method_helper_import("has_error_nodes"),
            Some("treeHasErrorNodes".to_string())
        );
    }

    #[test]
    fn ts_method_helper_import_returns_none_for_unknown() {
        assert!(ts_method_helper_import("some_unknown_method").is_none());
    }

    #[test]
    fn sanitize_filename_produces_expected_names() {
        let groups = [
            FixtureGroup {
                category: "basic tests".to_string(),
                fixtures: vec![],
            },
            FixtureGroup {
                category: "edge cases".to_string(),
                fixtures: vec![],
            },
        ];
        let names: Vec<String> = groups
            .iter()
            .map(|g| format!("{}.test.ts", sanitize_filename(&g.category)))
            .collect();
        assert_eq!(names, vec!["basic_tests.test.ts", "edge_cases.test.ts"]);
    }
}
