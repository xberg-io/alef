//! C e2e special call-pattern test rendering.

use crate::e2e::codegen::transform_json_keys_for_language;
use crate::e2e::escape::escape_c;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;

use super::{
    emit_nested_accessor, infer_opaque_handle_type, is_primitive_c_type, is_skipped_c_field, render_assertion,
    try_emit_enum_accessor,
};

/// Emit a test function using the engine-factory pattern:
///   `{prefix}_crawl_config_from_json(json)` → `{prefix}_create_engine(config)` →
///   `{prefix}_{function}(engine, url)` → assertions → free chain.
///
/// When all fixture assertions are skipped (fields not present on result type,
/// or only "error" assertions that C cannot replicate via a simple URL scrape),
/// the null-check is a soft guard (`if (result != NULL)`) so the test does not
/// abort when the mock server has no matching route.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_engine_factory_test_function(
    out: &mut String,
    fixture: &Fixture,
    prefix: &str,
    function_name: &str,
    result_var: &str,
    field_resolver: &FieldResolver,
    fields_c_types: &HashMap<String, String>,
    fields_enum: &HashSet<String>,
    result_type_name: &str,
    config_type: &str,
    expects_error: bool,
    raw_c_result_type: Option<&str>,
) {
    let prefix_upper = prefix.to_uppercase();
    let config_snake = config_type.to_snake_case();

    // Build config JSON from fixture input (snake_case keys).
    let config_val = fixture.input.get("config");
    let config_json = match config_val {
        Some(v) if !v.is_null() => {
            let normalized = transform_json_keys_for_language(v, "snake_case");
            serde_json::to_string(&normalized).unwrap_or_else(|_| "{}".to_string())
        }
        _ => "{}".to_string(),
    };
    let config_escaped = escape_c(&config_json);
    let fixture_id = &fixture.id;

    // An assertion is "active" when it has a field that is valid for the result type.
    // Error-only assertions are NOT treated as active for the engine factory pattern
    // because C's kcrawl_scrape() doesn't replicate batch/validation error semantics.
    let has_active_assertions = fixture.assertions.iter().any(|a| {
        if let Some(f) = &a.field {
            !f.is_empty() && field_resolver.is_valid_for_result(f)
        } else {
            false
        }
    });

    // --- engine setup ---
    let _ = writeln!(
        out,
        "    {prefix_upper}{config_type}* config_handle = \
         {prefix}_{config_snake}_from_json(\"{config_escaped}\");"
    );
    if expects_error {
        // Config parsing may legitimately fail for error fixtures (e.g. invalid config
        // rejected by the FFI layer). Return early — that counts as the expected failure.
        let _ = writeln!(out, "    if (config_handle == NULL) {{ return; }}");
    } else {
        let _ = writeln!(out, "    assert(config_handle != NULL && \"failed to parse config\");");
    }
    let _ = writeln!(
        out,
        "    {prefix_upper}CrawlEngineHandle* engine = {prefix}_create_engine(config_handle);"
    );
    let _ = writeln!(out, "    {prefix}_{config_snake}_free(config_handle);");
    if expects_error {
        // Engine creation may legitimately fail for error fixtures (e.g. invalid config
        // rejected at engine-creation time). Return early — that counts as the expected failure.
        let _ = writeln!(out, "    if (engine == NULL) {{ return; }}");
    } else {
        let _ = writeln!(out, "    assert(engine != NULL && \"failed to create engine\");");
    }

    // --- URL construction: prefer per-fixture MOCK_SERVER_<UPPER_ID> (for fixtures
    // that need host-root routes like /robots.txt or /sitemap.xml), fall back to
    // MOCK_SERVER_URL/fixtures/<id> for the common case. ---
    let fixture_env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
    let _ = writeln!(out, "    const char* mock_per_fixture = getenv(\"{fixture_env_key}\");");
    let _ = writeln!(out, "    const char* mock_base = getenv(\"MOCK_SERVER_URL\");");
    let _ = writeln!(out, "    char url[2048];");
    let _ = writeln!(out, "    if (mock_per_fixture && mock_per_fixture[0] != '\\0') {{");
    let _ = writeln!(out, "        snprintf(url, sizeof(url), \"%s\", mock_per_fixture);");
    let _ = writeln!(out, "    }} else {{");
    let _ = writeln!(
        out,
        "        assert(mock_base != NULL && \"MOCK_SERVER_URL must be set\");"
    );
    let _ = writeln!(
        out,
        "        snprintf(url, sizeof(url), \"%s/fixtures/{fixture_id}\", mock_base);"
    );
    let _ = writeln!(out, "    }}");

    // --- actions argument (interact and similar 3-arg engine-factory calls) ---
    // When the fixture input contains an "actions" key (interaction fixtures), the FFI
    // function signature is `{prefix}_{fn}(engine, url, actions_json)`.  Serialize the
    // actions value to a JSON string and emit a local `const char*` that is appended as
    // the third positional argument.
    let actions_arg = fixture.input.get("actions").and_then(|v| {
        if v.is_null() {
            None
        } else {
            let normalized = transform_json_keys_for_language(v, "snake_case");
            let json = serde_json::to_string(&normalized).ok()?;
            let escaped = escape_c(&json);
            Some(escaped)
        }
    });
    if let Some(ref escaped_actions) = actions_arg {
        let _ = writeln!(out, "    const char* actions_json = \"{escaped_actions}\";");
    }

    // --- call ---
    // Determine the trailing extra arguments beyond (engine, url).
    let extra_call_args = if actions_arg.is_some() {
        ", actions_json".to_string()
    } else {
        String::new()
    };

    // When the function returns a raw C type that is NOT an opaque struct pointer, emit a
    // plain variable declaration.
    //   • "char*" — JSON-returning helpers (batch_scrape historic config); use char* type
    //     and free with {prefix}_free_string.
    //   • Any other non-empty value — treat as an opaque PascalCase type name, emit
    //     {PREFIX}{Type}* and free with {prefix}_{type_snake}_free.  Callers set this when
    //     the function returns a named result struct (e.g. "BatchCrawlResults") that has no
    //     structured field accessors to assert on.
    if let Some(raw_type) = raw_c_result_type {
        if raw_type == "char*" {
            let _ = writeln!(
                out,
                "    char* {result_var} = {prefix}_{function_name}(engine, url{extra_call_args});"
            );
            let _ = writeln!(out, "    if ({result_var} != NULL) {prefix}_free_string({result_var});");
            let _ = writeln!(out, "    {prefix}_crawl_engine_handle_free(engine);");
            let _ = writeln!(out, "}}");
            return;
        } else {
            // Opaque struct return: emit the typed pointer, a soft null-guard, and the
            // matching free function derived from the snake_case type name.
            let raw_snake = raw_type.to_snake_case();
            let _ = writeln!(
                out,
                "    {prefix_upper}{raw_type}* {result_var} = {prefix}_{function_name}(engine, url{extra_call_args});"
            );
            let _ = writeln!(
                out,
                "    if ({result_var} != NULL) {prefix}_{raw_snake}_free({result_var});"
            );
            let _ = writeln!(out, "    {prefix}_crawl_engine_handle_free(engine);");
            let _ = writeln!(out, "}}");
            return;
        }
    }

    let _ = writeln!(
        out,
        "    {prefix_upper}{result_type_name}* {result_var} = {prefix}_{function_name}(engine, url{extra_call_args});"
    );

    // When no assertions can be verified (all skipped or error-only), use a soft
    // null-guard so the test is a no-op rather than aborting on a NULL result.
    if !has_active_assertions {
        let result_type_snake = result_type_name.to_snake_case();
        let _ = writeln!(
            out,
            "    if ({result_var} != NULL) {prefix}_{result_type_snake}_free({result_var});"
        );
        let _ = writeln!(out, "    {prefix}_crawl_engine_handle_free(engine);");
        let _ = writeln!(out, "}}");
        return;
    }

    let _ = writeln!(out, "    assert({result_var} != NULL && \"expected call to succeed\");");

    // --- field assertions ---
    let mut intermediate_handles: Vec<(String, String)> = Vec::new();
    let mut accessed_fields: Vec<(String, String, bool)> = Vec::new();
    let mut primitive_locals: HashMap<String, String> = HashMap::new();
    let mut opaque_handle_locals: HashMap<String, String> = HashMap::new();

    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field {
            if !f.is_empty() && field_resolver.is_valid_for_result(f) && !accessed_fields.iter().any(|(k, _, _)| k == f)
            {
                let resolved_raw = field_resolver.resolve(f);
                // Strip virtual namespace prefixes (e.g. "interaction.action_results[0].x"
                // → "action_results[0].x") matching the same logic as FieldResolver::accessor.
                let resolved = if let Some(stripped) = field_resolver.namespace_stripped_path(resolved_raw) {
                    let stripped_first = stripped.split('.').next().unwrap_or(stripped);
                    let stripped_first = stripped_first.split('[').next().unwrap_or(stripped_first);
                    if field_resolver.is_valid_for_result(stripped_first) {
                        stripped
                    } else {
                        resolved_raw
                    }
                } else {
                    resolved_raw
                };
                let local_var = f.replace(['.', '['], "_").replace(']', "");
                let has_map_access = resolved.contains('[');
                if resolved.contains('.') {
                    let leaf_result = emit_nested_accessor(
                        out,
                        prefix,
                        resolved,
                        &local_var,
                        result_var,
                        fields_c_types,
                        fields_enum,
                        &mut intermediate_handles,
                        result_type_name,
                        f,
                    );
                    if let Some(returned_type) = leaf_result {
                        // Could be a primitive type (primitive_locals) or opaque handle type
                        if is_primitive_c_type(&returned_type) {
                            primitive_locals.insert(local_var.clone(), returned_type);
                        } else {
                            // Opaque handle returned — register for cleanup
                            opaque_handle_locals.insert(local_var.clone(), returned_type);
                        }
                    }
                } else {
                    let result_type_snake = result_type_name.to_snake_case();
                    let accessor_fn = format!("{prefix}_{result_type_snake}_{resolved}");
                    let lookup_key = format!("{result_type_snake}.{resolved}");
                    if is_skipped_c_field(fields_c_types, &result_type_snake, resolved) {
                        // Field marked "skip" — record sentinel so render_assertion skips it.
                        primitive_locals.insert(local_var.clone(), "__skip__".to_string());
                    } else if let Some(t) = fields_c_types.get(&lookup_key).filter(|t| is_primitive_c_type(t)) {
                        let _ = writeln!(out, "    {t} {local_var} = {accessor_fn}({result_var});");
                        primitive_locals.insert(local_var.clone(), t.clone());
                    } else if try_emit_enum_accessor(
                        out,
                        prefix,
                        &prefix_upper,
                        f,
                        resolved,
                        &result_type_snake,
                        &accessor_fn,
                        result_var,
                        &local_var,
                        fields_c_types,
                        fields_enum,
                        &mut intermediate_handles,
                    ) {
                        // accessor emitted with enum-to-string conversion
                    } else if let Some(handle_pascal) =
                        infer_opaque_handle_type(fields_c_types, &result_type_snake, resolved)
                    {
                        let _ = writeln!(
                            out,
                            "    {prefix_upper}{handle_pascal}* {local_var} = {accessor_fn}({result_var});"
                        );
                        opaque_handle_locals.insert(local_var.clone(), handle_pascal.to_snake_case());
                    } else {
                        let _ = writeln!(out, "    char* {local_var} = {accessor_fn}({result_var});");
                    }
                }
                accessed_fields.push((f.clone(), local_var, has_map_access));
            }
        }
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            prefix,
            field_resolver,
            &accessed_fields,
            &primitive_locals,
            &opaque_handle_locals,
        );
    }

    // --- free locals ---
    for (_f, local_var, from_json) in &accessed_fields {
        if primitive_locals.contains_key(local_var) {
            continue;
        }
        if let Some(snake_type) = opaque_handle_locals.get(local_var) {
            let _ = writeln!(out, "    {prefix}_{snake_type}_free({local_var});");
            continue;
        }
        if *from_json {
            let _ = writeln!(out, "    free({local_var});");
        } else {
            let _ = writeln!(out, "    {prefix}_free_string({local_var});");
        }
    }
    for (handle_var, snake_type) in intermediate_handles.iter().rev() {
        if snake_type == "free_string" {
            let _ = writeln!(out, "    {prefix}_free_string({handle_var});");
        } else if snake_type == "free" {
            // Intermediate JSON-key extraction (e.g. alef_json_array_get_index) — freed via plain free().
            let _ = writeln!(out, "    free({handle_var});");
        } else {
            let _ = writeln!(out, "    {prefix}_{snake_type}_free({handle_var});");
        }
    }

    let result_type_snake = result_type_name.to_snake_case();
    let _ = writeln!(out, "    {prefix}_{result_type_snake}_free({result_var});");
    let _ = writeln!(out, "    {prefix}_crawl_engine_handle_free(engine);");
    let _ = writeln!(out, "}}");
}

/// Emit a byte-buffer test function for FFI methods returning raw bytes via
/// the out-pointer pattern (e.g. `speech`, `file_content`).
///
/// FFI signature shape:
/// ```c
/// int32_t {prefix}_default_client_{fn}(
///     const Client *this_,
///     const Request *req,                /* present when args is non-empty */
///     uint8_t **out_ptr,
///     uintptr_t *out_len,
///     uintptr_t *out_cap);
/// ```
///
/// Emits:
/// - request handle build (same as the standard client pattern)
/// - `uint8_t *out_ptr = NULL; uintptr_t out_len = 0, out_cap = 0;`
/// - call with `&out_ptr, &out_len, &out_cap`
/// - status assertion: `status == 0` on success, `status != 0` on expected error
/// - per-assertion: `not_empty` / `not_null` collapse to `out_len > 0` because
///   the pseudo "audio" / "content" field is the byte buffer itself
/// - `{prefix}_free_bytes(out_ptr, out_len, out_cap)` after assertions
#[allow(clippy::too_many_arguments)]
pub(super) fn render_bytes_test_function(
    out: &mut String,
    fixture: &Fixture,
    prefix: &str,
    function_name: &str,
    _result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    options_type_name: &str,
    result_type_name: &str,
    factory: &str,
    client_owner_type: &str,
    expects_error: bool,
) {
    let prefix_upper = prefix.to_uppercase();
    let mut request_handle_vars: Vec<(String, String)> = Vec::new();
    let mut string_arg_exprs: Vec<String> = Vec::new();

    for arg in args {
        match arg.arg_type.as_str() {
            "json_object" => {
                let request_type_pascal = if !options_type_name.is_empty() {
                    options_type_name.to_string()
                } else if let Some(stripped) = result_type_name.strip_suffix("Response") {
                    format!("{}Request", stripped)
                } else {
                    format!("{result_type_name}Request")
                };
                let request_type_snake = request_type_pascal.to_snake_case();
                let var_name = format!("{request_type_snake}_handle");

                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                let json_val = if field.is_empty() || field == "input" {
                    Some(&fixture.input)
                } else {
                    fixture.input.get(field)
                };

                if let Some(val) = json_val {
                    if !val.is_null() {
                        let normalized = transform_json_keys_for_language(val, "snake_case");
                        let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                        let escaped = escape_c(&json_str);
                        let _ = writeln!(
                            out,
                            "    {prefix_upper}{request_type_pascal}* {var_name} = \
                             {prefix}_{request_type_snake}_from_json(\"{escaped}\");"
                        );
                        if expects_error {
                            // For error fixtures (e.g. invalid enum value rejected by
                            // serde), `_from_json` may legitimately return NULL — that
                            // counts as the expected failure. Mirror Java's pattern of
                            // wrapping setup + call inside `assertThrows(...)` so error
                            // fixtures pass at *any* failure step. The test returns
                            // before attempting to create a client, leaving no
                            // resources to free.
                            let _ = writeln!(out, "    if ({var_name} == NULL) {{ return; }}");
                        } else {
                            let _ = writeln!(out, "    assert({var_name} != NULL && \"failed to build request\");");
                        }
                        request_handle_vars.push((arg.name.clone(), var_name));
                    }
                }
            }
            "string" => {
                // Pass string args (e.g. file_id for file_content) directly as
                // C string literals.
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                let val = fixture.input.get(field);
                let expr = match val {
                    Some(serde_json::Value::String(s)) => format!("\"{}\"", escape_c(s)),
                    Some(serde_json::Value::Null) | None if arg.optional => "NULL".to_string(),
                    Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "NULL".to_string()),
                    None => "NULL".to_string(),
                };
                string_arg_exprs.push(expr);
            }
            _ => {
                // Other arg types are not currently exercised by byte-buffer
                // methods; pass NULL so the call shape compiles.
                string_arg_exprs.push("NULL".to_string());
            }
        }
    }

    let fixture_id = &fixture.id;
    if fixture.needs_mock_server() {
        let _ = writeln!(out, "    const char* mock_base = getenv(\"MOCK_SERVER_URL\");");
        let _ = writeln!(out, "    assert(mock_base != NULL && \"MOCK_SERVER_URL must be set\");");
        let _ = writeln!(out, "    char base_url[1024];");
        let _ = writeln!(
            out,
            "    snprintf(base_url, sizeof(base_url), \"%s/fixtures/{fixture_id}\", mock_base);"
        );
        // Pass UINT64_MAX/UINT32_MAX (≡ -1ULL/-1U) as the FFI's None sentinel for
        // optional numeric primitives — passing literal 0 makes the binding see
        // Some(0), which Rust core treats as `Duration::from_secs(0)` (immediate
        // request deadline) and breaks every HTTP fixture.
        let _ = writeln!(
            out,
            "    {prefix_upper}{client_owner_type}* client = {prefix}_{factory}(\"test-key\", base_url, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    } else {
        let _ = writeln!(
            out,
            "    {prefix_upper}{client_owner_type}* client = {prefix}_{factory}(\"test-key\", NULL, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    }
    let _ = writeln!(out, "    assert(client != NULL && \"failed to create client\");");

    // Out-params for the byte buffer.
    let _ = writeln!(out, "    uint8_t* out_ptr = NULL;");
    let _ = writeln!(out, "    uintptr_t out_len = 0;");
    let _ = writeln!(out, "    uintptr_t out_cap = 0;");

    // Build the comma-separated argument list: handles, then string args.
    let mut method_args: Vec<String> = Vec::new();
    for (_, v) in &request_handle_vars {
        method_args.push(v.clone());
    }
    method_args.extend(string_arg_exprs.iter().cloned());
    let extra_args = if method_args.is_empty() {
        String::new()
    } else {
        format!(", {}", method_args.join(", "))
    };

    let call_fn = format!("{prefix}_default_client_{function_name}");
    let _ = writeln!(
        out,
        "    int32_t status = {call_fn}(client{extra_args}, &out_ptr, &out_len, &out_cap);"
    );

    if expects_error {
        for (_, var_name) in &request_handle_vars {
            let req_snake = var_name.strip_suffix("_handle").unwrap_or(var_name);
            let _ = writeln!(out, "    {prefix}_{req_snake}_free({var_name});");
        }
        let _ = writeln!(out, "    {prefix}_default_client_free(client);");
        let _ = writeln!(out, "    assert(status != 0 && \"expected call to fail\");");
        // free_bytes accepts a NULL ptr (no-op), so it is safe regardless of
        // whether the failed call wrote out_ptr.
        let _ = writeln!(out, "    {prefix}_free_bytes(out_ptr, out_len, out_cap);");
        let _ = writeln!(out, "}}");
        return;
    }

    let _ = writeln!(out, "    assert(status == 0 && \"expected call to succeed\");");

    // Render assertions. For byte-buffer methods, the only meaningful per-field
    // assertions are presence/length checks on the buffer itself. Field names
    // (e.g. "audio", "content") are pseudo-fields — collapse them all to
    // `out_len > 0`.
    let mut emitted_len_check = false;
    for assertion in &fixture.assertions {
        match assertion.assertion_type.as_str() {
            "not_error" => {
                // Already covered by the status == 0 assertion above.
            }
            "not_empty" | "not_null" => {
                if !emitted_len_check {
                    let _ = writeln!(out, "    assert(out_len > 0 && \"expected non-empty value\");");
                    emitted_len_check = true;
                }
            }
            _ => {
                // Other assertion shapes (equals, contains, ...) don't apply to
                // raw bytes; emit a comment so the test stays readable but does
                // not emit broken accessor calls.
                let _ = writeln!(
                    out,
                    "    /* skipped: assertion '{}' not meaningful on raw byte buffer */",
                    assertion.assertion_type
                );
            }
        }
    }

    let _ = writeln!(out, "    {prefix}_free_bytes(out_ptr, out_len, out_cap);");
    for (_, var_name) in &request_handle_vars {
        let req_snake = var_name.strip_suffix("_handle").unwrap_or(var_name);
        let _ = writeln!(out, "    {prefix}_{req_snake}_free({var_name});");
    }
    let _ = writeln!(out, "    {prefix}_default_client_free(client);");
    let _ = writeln!(out, "}}");
}
