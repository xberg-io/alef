//! Test file and test case rendering for TypeScript e2e tests.

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
    let (needs_cache_isolation, has_configure) = detect_cache_isolation_needs(fixtures, e2e_config);

    let import_vitest = if needs_cache_isolation && has_configure {
        "import { describe, expect, it, beforeAll, afterAll } from 'vitest';"
    } else {
        "import { describe, expect, it } from 'vitest';"
    };

    let has_non_http_fixtures = fixtures.iter().any(|f| !f.is_http_test() && !f.assertions.is_empty());

    // Extract nested_types and enum_fields from the call override if available.
    let override_config = e2e_config.call.overrides.get(lang);
    let nested_types = override_config.map(|o| o.nested_types.clone()).unwrap_or_default();
    let enum_fields = override_config.map(|o| o.enum_fields.clone()).unwrap_or_default();

    // Per-fixture wasm/node overrides may add their own options_type / nested_types /
    // enum_fields (each call exposes a different request struct in WASM, e.g.
    // `WasmEmbeddingRequest` vs `WasmChatCompletionRequest`). Aggregate every class
    // referenced across this file's fixtures so the import line covers them all.
    // The global `options_type` parameter remains the default fallback when a
    // per-call override is absent.
    let mut all_options_types: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut all_nested_types: std::collections::HashMap<String, String> = nested_types.clone();
    let mut all_enum_fields: std::collections::HashMap<String, String> = enum_fields.clone();
    if let Some(opts) = options_type {
        all_options_types.insert(opts.to_string());
    }
    for fixture in fixtures.iter() {
        let cc = e2e_config.resolve_call(fixture.call.as_deref());
        if let Some(o) = cc.overrides.get(lang) {
            if let Some(opts) = &o.options_type {
                all_options_types.insert(opts.clone());
            }
            for (k, v) in &o.nested_types {
                all_nested_types.entry(k.clone()).or_insert_with(|| v.clone());
            }
            for (k, v) in &o.enum_fields {
                all_enum_fields.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }

    let needs_options_import = !all_options_types.is_empty()
        && fixtures.iter().any(|f| {
            let cc = e2e_config.resolve_call(f.call.as_deref());
            cc.args.iter().any(|arg| {
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

    let mut import_modules = String::new();
    let mut import_node_fs = String::new();

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
        if needs_options_import {
            if lang == "node" {
                // ConversionOptions is a TypeScript interface — type-only import.
                // No Update class exists; options are constructed as plain object literals.
                for opts_type in &all_options_types {
                    imports.push(format!("type {opts_type}"));
                }
            } else {
                // WASM: value import needed for runtime construction. The
                // alef-backend-wasm codegen does not emit `*Update` builder
                // classes, so we construct the main type directly via its
                // all-optional positional constructor and then assign each
                // present field through generated setters. Nested types use
                // the same pattern. See `ts_builder_expression_inner`.
                for opts_type in &all_options_types {
                    if !imports.contains(opts_type) {
                        imports.push(opts_type.clone());
                    }
                }
                for nested_type in all_nested_types.values() {
                    if !imports.contains(nested_type) {
                        imports.push(nested_type.clone());
                    }
                }
                // Also import enum types referenced in this test file
                for enum_type in all_enum_fields.values() {
                    if !imports.contains(enum_type) {
                        imports.push(enum_type.clone());
                    }
                }
            }
            let imports_str = imports.join(", ");
            import_modules = format!("import {{ {imports_str} }} from '{pkg_name}';");
        } else {
            let imports_str = imports.join(", ");
            import_modules = format!("import {{ {imports_str} }} from '{pkg_name}';");
        }

        if needs_cache_isolation && has_configure {
            import_node_fs = "import { mkdtempSync, rmSync } from 'node:fs';\nimport { join } from 'node:path';\nimport { tmpdir } from 'node:os';".to_string();
        }
    }

    // Build helper functions string
    let helper_functions = if has_non_http_fixtures {
        crate::template_env::render("typescript/helpers.jinja", minijinja::context! {})
    } else {
        String::new()
    };

    // Build cache isolation setup
    let mut cache_isolation_setup = String::new();
    if needs_cache_isolation && has_configure {
        emit_cache_isolation_setup(&mut cache_isolation_setup);
    }

    // Build fixtures body
    let mut fixtures_body = String::new();
    for (i, fixture) in fixtures.iter().enumerate() {
        if fixture.is_http_test() {
            render_http_test_case(&mut fixtures_body, fixture);
        } else {
            render_test_case(
                &mut fixtures_body,
                fixture,
                client_factory,
                options_type,
                field_resolver,
                e2e_config,
                lang,
                &nested_types,
                &enum_fields,
            );
        }
        if i + 1 < fixtures.len() {
            fixtures_body.push('\n');
        }
    }

    let ctx = minijinja::context! {
        header => hash::header(CommentStyle::DoubleSlash),
        import_vitest => import_vitest,
        import_modules => import_modules,
        import_node_fs => import_node_fs,
        helper_functions => helper_functions,
        category => category,
        cache_isolation_setup => cache_isolation_setup,
        fixtures_body => fixtures_body,
    };
    crate::template_env::render("typescript/test_file.jinja", ctx)
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
        let rendered = crate::template_env::render(
            "typescript/http_test_skip_101.jinja",
            minijinja::context! {
                test_name => test_name,
                description => description,
            },
        );
        out.push_str(&rendered);
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
    let init_str = init_entries.join(", ");

    let status = http.expected_response.status_code;

    // Determine body type and prepare context
    let (has_text_body, text_body) = if let Some(expected_body) = &http.expected_response.body {
        if !(expected_body.is_null() || expected_body.is_string() && expected_body.as_str() == Some("")) {
            if let serde_json::Value::String(s) = expected_body {
                (true, escape_js(s))
            } else {
                (false, String::new())
            }
        } else {
            (false, String::new())
        }
    } else {
        (false, String::new())
    };

    let (has_json_body, json_val) = if let Some(expected_body) = &http.expected_response.body {
        if !(expected_body.is_null() || expected_body.is_string() && expected_body.as_str() == Some("")) {
            if let serde_json::Value::String(_) = expected_body {
                (false, String::new())
            } else {
                (true, json_to_js(expected_body))
            }
        } else {
            (false, String::new())
        }
    } else {
        (false, String::new())
    };

    let (has_partial_body, partial_body_checks) = if let Some(partial) = &http.expected_response.body_partial {
        if let Some(obj) = partial.as_object() {
            let checks: Vec<minijinja::Value> = obj
                .iter()
                .map(|(key, val)| {
                    minijinja::context! {
                        key => escape_js(key),
                        js_val => json_to_js(val),
                    }
                })
                .collect();
            (true, checks)
        } else {
            (false, Vec::new())
        }
    } else {
        (false, Vec::new())
    };

    // Build header assertions
    let mut header_assertions: Vec<minijinja::Value> = Vec::new();
    for (header_name, header_value) in &http.expected_response.headers {
        let lower_name = header_name.to_lowercase();
        if lower_name == "content-encoding" {
            continue;
        }
        let escaped_name = escape_js(&lower_name);
        let (assertion_type, value) = match header_value.as_str() {
            "<<present>>" => ("present", String::new()),
            "<<absent>>" => ("absent", String::new()),
            "<<uuid>>" => ("uuid", String::new()),
            exact => ("exact", escape_js(exact)),
        };
        header_assertions.push(minijinja::context! {
            name => escaped_name,
            assertion_type => assertion_type,
            value => value,
        });
    }

    // Build validation error assertions
    let body_has_content = matches!(&http.expected_response.body, Some(v)
        if !(v.is_null() || (v.is_string() && v.as_str() == Some(""))));
    let (has_validation_errors, validation_errors) =
        if let Some(validation_errors) = &http.expected_response.validation_errors {
            if !validation_errors.is_empty() && !body_has_content {
                let errors: Vec<minijinja::Value> = validation_errors
                    .iter()
                    .map(|ve| {
                        let loc_js: Vec<String> = ve.loc.iter().map(|s| format!("\"{}\"", escape_js(s))).collect();
                        let loc_str = loc_js.join(", ");
                        let expanded_msg = expand_fixture_templates(&ve.msg);
                        let escaped_msg = escape_js(&expanded_msg);
                        minijinja::context! {
                            loc_js => loc_str,
                            escaped_msg => escaped_msg,
                        }
                    })
                    .collect();
                (true, errors)
            } else {
                (false, Vec::new())
            }
        } else {
            (false, Vec::new())
        };

    let ctx = minijinja::context! {
        test_name => test_name,
        description => description,
        method => method,
        init_str => init_str,
        fixture_id => fixture_id,
        expected_status => status,
        has_text_body => has_text_body,
        text_body => text_body,
        has_json_body => has_json_body,
        json_val => json_val,
        has_partial_body => has_partial_body,
        partial_body_checks => partial_body_checks,
        header_assertions => header_assertions,
        has_validation_errors => has_validation_errors,
        validation_errors => validation_errors,
    };
    let rendered = crate::template_env::render("typescript/http_test.jinja", ctx);
    out.push_str(&rendered);
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
    nested_types: &std::collections::HashMap<String, String>,
    enum_fields: &std::collections::HashMap<String, String>,
) {
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let function_name = resolve_node_function_name(call_config);
    let result_var = &call_config.result_var;
    let call_is_async = call_config.r#async;
    let args = &call_config.args;
    let result_is_simple =
        call_config.result_is_simple || call_config.overrides.get(lang).is_some_and(|o| o.result_is_simple);

    // Resolve per-fixture wasm/node override fields (options_type, bigint_fields,
    // nested_types, enum_fields). Per-call overrides win over the file-level
    // default; missing fields fall back to the file-level default. WASM/wasm-bindgen
    // is the primary consumer of `bigint_fields` (u64/i64 setters reject Number).
    let per_call_override = call_config.overrides.get(lang);
    let effective_options_type: Option<String> = per_call_override
        .and_then(|o| o.options_type.clone())
        .or_else(|| options_type.map(|s| s.to_string()));
    let mut effective_nested_types: std::collections::HashMap<String, String> = nested_types.clone();
    if let Some(o) = per_call_override {
        for (k, v) in &o.nested_types {
            effective_nested_types.insert(k.clone(), v.clone());
        }
    }
    let mut effective_enum_fields: std::collections::HashMap<String, String> = enum_fields.clone();
    if let Some(o) = per_call_override {
        for (k, v) in &o.enum_fields {
            effective_enum_fields.insert(k.clone(), v.clone());
        }
    }
    let global_bigint_fields: Vec<String> = e2e_config
        .call
        .overrides
        .get(lang)
        .map(|o| o.bigint_fields.clone())
        .unwrap_or_default();
    let mut effective_bigint_fields: std::collections::BTreeSet<String> = global_bigint_fields.into_iter().collect();
    if let Some(o) = per_call_override {
        for f in &o.bigint_fields {
            effective_bigint_fields.insert(f.clone());
        }
    }

    // Force test to async if we need to read files for bytes args
    let test_is_async = call_is_async || has_bytes_file_reads(&fixture.input, args);

    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.replace('\'', "\\'");
    let async_kw = if test_is_async { "async " } else { "" };
    let await_kw = if call_is_async { "await " } else { "" };

    let (mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        effective_options_type.as_deref(),
        &fixture.id,
        &effective_nested_types,
        lang,
        &effective_enum_fields,
        &effective_bigint_fields,
    );

    let mut visitor_arg = String::new();
    if let Some(visitor_spec) = &fixture.visitor {
        visitor_arg = build_typescript_visitor(&mut setup_lines, visitor_spec);
    }

    let final_args = if visitor_arg.is_empty() {
        args_str
    } else if lang == "wasm" || lang == "node" {
        // WASM and Node: visitor must be merged into the options object (2nd arg) — both
        // bindings expose convert(html, options?) and ignore any additional positional
        // arguments, so 'append the visitor as a 3rd arg' silently dropped the visitor.
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
        } else if let Some(stripped) = args_str.strip_suffix(", undefined") {
            // After the `{} as OptionsType` → `undefined` change, the empty-options
            // tail no longer carries a cast for us to splice into. Replace the trailing
            // undefined with the visitor-bearing options object.
            format!("{stripped}, {{ visitor: {visitor_arg} }}")
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
    let is_skipped = fixture.assertions.is_empty();

    // Build client setup
    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    let client_setup = if let Some(factory) = client_factory {
        if has_mock {
            format!("const client = {factory}('test-key', {base_url_expr});")
        } else if let Some(var) = api_key_var {
            // Live-API tests: skip when the env var isn't set so the suite can run
            // without real credentials, matching the python codegen's pattern.
            format!(
                "const apiKey = process.env.{var};\n    \
                 if (!apiKey) {{\n        \
                     return;\n    \
                 }}\n    \
                 const client = {factory}(apiKey);"
            )
        } else {
            format!("const client = {factory}('test-key', {base_url_expr});")
        }
    } else {
        String::new()
    };

    // Build assertions body
    let mut assertions_body = String::new();
    for assertion in &fixture.assertions {
        if assertion.assertion_type == "not_error" && !call_config.returns_result {
            continue;
        }
        render_assertion(
            &mut assertions_body,
            assertion,
            result_var,
            field_resolver,
            result_is_simple,
        );
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

    let ctx = minijinja::context! {
        test_name => test_name,
        description => description,
        async_kw => async_kw,
        client_setup => client_setup,
        setup_lines => setup_lines,
        call_expr => call_expr,
        has_usable_assertion => has_usable_assertion,
        result_var => result_var,
        await_kw => await_kw,
        assertions_body => assertions_body,
        expects_error => expects_error,
        is_skipped => is_skipped,
    };
    let rendered = crate::template_env::render("typescript/test_function.jinja", ctx);
    out.push_str(&rendered);
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

/// Build a TypeScript expression to construct an options object.
///
/// Node: ConversionOptions is a TypeScript interface — returns a plain object literal
/// with a type assertion (`{ key: val } as TypeName`). No Update class or fromUpdate().
///
/// WASM: alef-backend-wasm does not emit `*Update` builder classes, so we
/// instantiate the main type directly. Every wasm-bindgen-emitted struct
/// exposes an all-optional positional constructor (`new T()`) plus per-field
/// setters, so we build the value with `new T()` followed by setter
/// assignments wrapped in an IIFE so the expression can be inlined as a
/// function argument. Nested object values follow the same pattern.
fn ts_builder_expression(
    obj: &serde_json::Map<String, serde_json::Value>,
    type_name: &str,
    nested_types: &std::collections::HashMap<String, String>,
    lang: &str,
    enum_fields: &std::collections::HashMap<String, String>,
    bigint_fields: &std::collections::BTreeSet<String>,
) -> String {
    ts_builder_expression_inner(obj, type_name, nested_types, lang, enum_fields, bigint_fields)
}

/// Convert a JS numeric literal expression to a BigInt-compatible literal
/// (`123n`, `-7n`) for wasm-bindgen `u64`/`i64` setters which reject Number.
/// Non-integer or non-numeric expressions are wrapped in `BigInt(...)` so the
/// runtime conversion still happens.
fn to_bigint_literal(value_expr: &str) -> String {
    let trimmed = value_expr.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        return format!("{trimmed}n");
    }
    if let Some(rest) = trimmed.strip_prefix('-') {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            return format!("-{rest}n");
        }
    }
    format!("BigInt({trimmed})")
}

fn ts_builder_expression_inner(
    obj: &serde_json::Map<String, serde_json::Value>,
    type_name: &str,
    nested_types: &std::collections::HashMap<String, String>,
    lang: &str,
    enum_fields: &std::collections::HashMap<String, String>,
    bigint_fields: &std::collections::BTreeSet<String>,
) -> String {
    if lang == "node" {
        let mut fields = Vec::new();
        for (key, val) in obj {
            let camel_key = snake_to_camel(key);
            let field_expr = match val {
                serde_json::Value::Object(_) => json_to_js_camel(val),
                _ => json_to_js(val),
            };
            fields.push(format!("{camel_key}: {field_expr}"));
        }
        let obj_literal = format!("{{ {} }}", fields.join(", "));
        return format!("{obj_literal} as {type_name}");
    }

    // WASM path: construct the main type directly via its no-arg constructor
    // (every wasm-bindgen-emitted struct exposes an all-optional positional
    // ctor + per-field setters). Nested object values are constructed
    // recursively the same way.
    let mut stmts: Vec<String> = vec![format!("const _u = new {type_name}();")];
    for (key, val) in obj {
        let camel_key = snake_to_camel(key);
        let is_bigint = bigint_fields.contains(&camel_key) || bigint_fields.contains(key);
        if let serde_json::Value::Object(nested_obj) = val {
            if let Some(nested_type) = nested_types.get(key.as_str()) {
                let nested_expr = ts_builder_expression_inner(
                    nested_obj,
                    nested_type,
                    nested_types,
                    lang,
                    enum_fields,
                    bigint_fields,
                );
                stmts.push(format!("_u.{camel_key} = {nested_expr};"));
            } else {
                stmts.push(format!("_u.{camel_key} = {};", json_to_js_camel(val)));
            }
        } else if let Some(enum_type) = enum_fields.get(key.as_str()) {
            // This is an enum field — generate EnumType.EnumValue
            if let serde_json::Value::String(s) = val {
                stmts.push(format!("_u.{camel_key} = {enum_type}.{};", s));
            } else {
                // Non-string enum value, just use json_to_js
                stmts.push(format!("_u.{camel_key} = {};", json_to_js(val)));
            }
        } else if is_bigint {
            // wasm-bindgen u64/i64 setters require BigInt. Plain numeric
            // literals must be suffixed with `n`; non-literal numeric
            // values are wrapped in `BigInt(...)`.
            let raw = json_to_js(val);
            stmts.push(format!("_u.{camel_key} = {};", to_bigint_literal(&raw)));
        } else {
            stmts.push(format!("_u.{camel_key} = {};", json_to_js(val)));
        }
    }

    stmts.push("return _u;".to_string());
    let body = stmts.join(" ");
    format!("(() => {{ {body} }})()")
}

#[allow(clippy::too_many_arguments)]
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[ArgMapping],
    options_type: Option<&str>,
    fixture_id: &str,
    nested_types: &std::collections::HashMap<String, String>,
    lang: &str,
    enum_fields: &std::collections::HashMap<String, String>,
    bigint_fields: &std::collections::BTreeSet<String>,
) -> (Vec<String>, String) {
    if args.is_empty() {
        return (Vec::new(), json_to_js(input));
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    // Check if any later arg (after current) is a json_object that will get a default value
    // (needed to insert undefineds as placeholders for earlier missing optional args)
    fn has_later_json_object_default(args: &[ArgMapping], from_idx: usize, input: &serde_json::Value) -> bool {
        args[from_idx..].iter().any(|arg| {
            if arg.arg_type != "json_object" || !arg.optional {
                return false;
            }
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            input.get(field).is_none() || input.get(field).map(|v| v.is_null()).unwrap_or(true)
        })
    }

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
                // For optional json_object args, pass `undefined` so we keep argument
                // positions intact without needing a placeholder value. The previous
                // `{} as OptionsType` pattern broke wasm-bindgen, where the runtime
                // `instanceof` check rejected plain object literals — wasm exposes
                // options as opaque classes, not interfaces.
                if arg.arg_type == "json_object"
                    || has_later_arg_value(args, idx + 1, input)
                    || has_later_json_object_default(args, idx + 1, input)
                {
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
                        // Object value with known options type — construct properly for wasm-bindgen.
                        if v.is_object() && v.as_object().is_some_and(|o| o.is_empty()) {
                            // Empty options: pass undefined so wasm-bindgen's instanceof
                            // guard accepts the call (a `{}` cast produces a plain literal
                            // that fails the runtime class check).
                            parts.push("undefined".to_string());
                        } else if let Some(obj) = v.as_object() {
                            // Build TypeScript code to construct the options object properly,
                            // handling nested types via their static factory methods.
                            let ts_code =
                                ts_builder_expression(obj, opts_type, nested_types, lang, enum_fields, bigint_fields);
                            parts.push(ts_code);
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

/// Detect if cache isolation is needed: checks if any fixture calls `cleanCache`
/// and if a `configure` function is available.
/// Returns (has_clean_cache, has_configure).
fn detect_cache_isolation_needs(fixtures: &[&Fixture], e2e_config: &E2eConfig) -> (bool, bool) {
    let has_clean_cache = fixtures.iter().any(|fixture| {
        let call_config = e2e_config.resolve_call(fixture.call.as_deref());
        resolve_node_function_name(call_config) == "cleanCache"
    });

    let has_configure = e2e_config
        .calls
        .iter()
        .any(|(_, call_config)| resolve_node_function_name(call_config) == "configure")
        || resolve_node_function_name(&e2e_config.call) == "configure";

    (has_clean_cache, has_configure)
}

/// Emit the cache isolation setup code (beforeAll/afterAll blocks).
fn emit_cache_isolation_setup(out: &mut String) {
    let rendered = crate::template_env::render("typescript/cache_isolation_setup.jinja", minijinja::context! {});
    out.push_str(&rendered);
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
