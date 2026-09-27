//! C e2e special call-pattern test rendering.

use crate::e2e::codegen::mock_assertions::is_mock_virtual_field;
use crate::e2e::codegen::transform_json_keys_for_language;
use crate::e2e::escape::escape_c;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;

use super::{
    FieldConfigSources, LeafFieldCheck, c_optional_sentinel, classify_nested_leaf, emit_nested_accessor,
    ensure_leaf_field_exists, infer_opaque_handle_type, is_primitive_c_type, is_skipped_c_field, render_assertion,
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
    type_defs: &[crate::core::ir::TypeDef],
    config_sources: &FieldConfigSources,
) -> anyhow::Result<()> {
    // cbindgen's `[export] prefix` (shouty-snake), not a bare uppercase — see
    // `c_consumer::export_type_prefix`. ~keep
    let prefix_upper = crate::codegen::c_consumer::export_type_prefix(prefix);
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
    // `mock.*` (alef issue #443) is never "valid for result type" -- it targets the mock
    // server's own request log, not a field of the result -- so without the `is_mock_virtual_field`
    // arm, a fixture whose ONLY assertion is `mock.*` would measure zero active assertions and
    // take the soft-null-guard branch below, which renders no assertions at all: a silent full
    // drop, worse than `FieldSkip::NotAvailableOnResultType`. ~keep
    let has_active_assertions = fixture.assertions.iter().any(|a| {
        if let Some(f) = &a.field {
            !f.is_empty() && (field_resolver.is_valid_for_result(f) || is_mock_virtual_field(f))
        } else {
            false
        }
    });

    // --- engine setup ---
    let _ = writeln!(
        out,
        "    {prefix_upper}AlefHandle config_handle = \
         {prefix}_{config_snake}_from_json(\"{config_escaped}\");"
    );
    if expects_error {
        // Config parsing may legitimately fail for error fixtures (e.g. invalid config
        // rejected by the FFI layer). Return early — that counts as the expected failure.
        let _ = writeln!(out, "    if (config_handle == 0) {{ ALEF_TEST_PASS(); }}");
    } else {
        let _ = writeln!(out, "    assert(config_handle != 0 && \"failed to parse config\");");
    }
    let _ = writeln!(
        out,
        "    {prefix_upper}AlefHandle engine = {prefix}_create_engine(config_handle);"
    );
    let _ = writeln!(out, "    {prefix}_{config_snake}_free(config_handle);");
    if expects_error {
        // Engine creation may legitimately fail for error fixtures (e.g. invalid config
        // rejected at engine-creation time). Return early — that counts as the expected failure.
        let _ = writeln!(out, "    if (engine == 0) {{ ALEF_TEST_PASS(); }}");
    } else {
        let _ = writeln!(out, "    assert(engine != 0 && \"failed to create engine\");");
    }

    // --- URL construction ---
    // ~keep This emitter is convention-based: it has no `ArgMapping`, so unlike the
    // other backends it cannot consult a `mock_url` arg's declared field and reads
    // `input.url` directly, the same way it already reads `input.config` and
    // `input.actions` above. `url` stays a `char[2048]` in both branches so nothing
    // downstream has to care which one produced it.
    //
    // Batch/list fixtures (e.g. `batch_crawl_*`) carry no `url` key at all -- their
    // addresses live in a list field (`batch_urls`, aliased via `resolve_urls_field`
    // the same way every other backend's `mock_url_list` handling resolves it). The
    // engine-factory ABI takes a single positional `url`, and C's own comment above
    // (`kcrawl_scrape() doesn't replicate batch/validation error semantics`) already
    // establishes that this pattern only ever smoke-tests one address for these
    // fixtures, so the first list entry is that address. Falling through the shared
    // `resolve_urls_field`/`preserved_url_list` seam here (rather than re-deriving the
    // `batch_urls` alias locally) keeps the alias list defined in exactly one place.
    let urls_value = crate::e2e::codegen::resolve_urls_field(&fixture.input, "input.urls");
    let preserved_urls = crate::e2e::codegen::preserved_url_list(fixture.preserve_input_urls, urls_value);
    let preserved_url = crate::e2e::codegen::preserved_url_literal(
        fixture.preserve_input_urls,
        crate::e2e::codegen::resolve_field(&fixture.input, "input.url"),
    )
    .or_else(|| preserved_urls.as_ref().and_then(|urls| urls.first().copied()));
    // ~keep alef #185: `batch_urls: []` is a deliberate empty-array fixture (testing the
    // empty-input error path), not an unset field. `preserved_url_list` returns
    // `Some(vec![])` for it -- distinct from `None`, which means either preservation is
    // off or the field was never declared at all. Treating the two the same sent an
    // empty, on-purpose list through the `getenv("MOCK_SERVER_URL")` branch below, which
    // has nothing to do with the fixture and which the published-snippet leak guard
    // rejects outright. There is no address to leak here, so render the empty list
    // literally: an empty `url` string, with no mock-harness scaffolding at all.
    let declared_empty_list = matches!(&preserved_urls, Some(urls) if urls.is_empty());
    if let Some(url) = preserved_url {
        // The fixture's own address is the subject of the test; the mock server
        // lookup is skipped entirely so no env var can override it.
        let url_escaped = escape_c(url);
        let _ = writeln!(out, "    char url[2048];");
        let _ = writeln!(out, "    snprintf(url, sizeof(url), \"%s\", \"{url_escaped}\");");
    } else if declared_empty_list {
        let _ = writeln!(out, "    char url[2048];");
        let _ = writeln!(out, "    snprintf(url, sizeof(url), \"%s\", \"\");");
    } else {
        // Prefer per-fixture MOCK_SERVER_<UPPER_ID> (for fixtures that need host-root
        // routes like /robots.txt or /sitemap.xml), fall back to
        // MOCK_SERVER_URL/fixtures/<id> for the common case.
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
    }

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
            return Ok(());
        } else {
            // Opaque struct return: emit the typed pointer, a soft null-guard, and the
            // matching free function derived from the snake_case type name.
            let raw_snake = raw_type.to_snake_case();
            let _ = writeln!(
                out,
                "    {prefix_upper}AlefHandle {result_var} = {prefix}_{function_name}(engine, url{extra_call_args});"
            );
            let _ = writeln!(
                out,
                "    if ({result_var} != 0) {prefix}_{raw_snake}_free({result_var});"
            );
            let _ = writeln!(out, "    {prefix}_crawl_engine_handle_free(engine);");
            let _ = writeln!(out, "}}");
            return Ok(());
        }
    }

    let _ = writeln!(
        out,
        "    {prefix_upper}AlefHandle {result_var} = {prefix}_{function_name}(engine, url{extra_call_args});"
    );

    // When no assertions can be verified (all skipped or error-only), use a soft
    // null-guard so the test is a no-op rather than aborting on a NULL result.
    if !has_active_assertions {
        let result_type_snake = result_type_name.to_snake_case();
        let _ = writeln!(
            out,
            "    if ({result_var} != 0) {prefix}_{result_type_snake}_free({result_var});"
        );
        let _ = writeln!(out, "    {prefix}_crawl_engine_handle_free(engine);");
        let _ = writeln!(out, "}}");
        return Ok(());
    }

    let _ = writeln!(out, "    assert({result_var} != 0 && \"expected call to succeed\");");

    // --- field assertions ---
    let mut intermediate_handles: Vec<(String, String)> = Vec::new();
    let mut accessed_fields: Vec<(String, String, bool)> = Vec::new();
    let mut primitive_locals: HashMap<String, String> = HashMap::new();
    let mut opaque_handle_locals: HashMap<String, String> = HashMap::new();
    // `field[].key` wildcard leaves: local_var -> (array json var, key to extract per element).
    // No scalar C local exists for these; `render_assertion` renders a per-element quantifier
    // from this instead. See `collection_wildcard.rs`.
    let mut wildcard_locals: HashMap<String, (String, String)> = HashMap::new();

    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field
            && !f.is_empty()
            // `field_resolver.is_valid_for_result` is permissive by default -- with no
            // `result_fields`/IR anchoring configured (the common case) it returns `true` for
            // ANY name, `mock.*` (alef issue #443) included, so it alone does not keep `mock.*`
            // out of accessor-chain extraction. Caught by `mock_capture_regression_tests`
            // exercising a realistically-permissive resolver instead of an empty one. ~keep
            && !is_mock_virtual_field(f)
            && field_resolver.is_valid_for_result(f)
            && !accessed_fields.iter().any(|(k, _, _)| k == f)
        {
            // Strips virtual namespace prefixes (e.g. "interaction.action_results[0].x"
            // → "action_results[0].x") before building the accessor chain.
            let resolved = field_resolver.result_relative_path(f);
            let resolved = resolved.as_ref();
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
                    type_defs,
                    config_sources,
                )?;
                if let Some(outcome) = leaf_result {
                    classify_nested_leaf(
                        outcome,
                        &local_var,
                        &mut primitive_locals,
                        &mut opaque_handle_locals,
                        &mut wildcard_locals,
                    );
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
                        "    {prefix_upper}AlefHandle {local_var} = {accessor_fn}({result_var});"
                    );
                    opaque_handle_locals.insert(local_var.clone(), handle_pascal.to_snake_case());
                } else {
                    // See `test_function.rs`: a single-segment `resolved` may be the residue
                    // of namespace stripping rather than a flat field, and the accessor name
                    // built from it is a guess until the IR confirms it. ~keep
                    ensure_leaf_field_exists(LeafFieldCheck {
                        prefix,
                        accessor_fn: &accessor_fn,
                        resolved,
                        raw_field: f,
                        segment: resolved,
                        parent_snake_type: &result_type_snake,
                        parent_is_ir_type: type_defs.iter().any(|type_def| type_def.name == result_type_name),
                        declared_in_fields_c_types: fields_c_types.contains_key(&lookup_key),
                        result_type_name,
                        type_defs,
                        result_fields_source: &config_sources.result_fields,
                        fields_source: &config_sources.fields,
                    })?;
                    let _ = writeln!(out, "    char* {local_var} = {accessor_fn}({result_var});");
                }
            }
            accessed_fields.push((f.clone(), local_var, has_map_access));
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
            &wildcard_locals,
        );
    }

    // --- free locals ---
    for (_f, local_var, from_json) in &accessed_fields {
        if primitive_locals.contains_key(local_var) {
            continue;
        }
        // No scalar local was ever declared for a wildcard leaf — the array json var it
        // reads is freed separately, below, via `intermediate_handles`.
        if wildcard_locals.contains_key(local_var) {
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
    Ok(())
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
    _client_owner_type: &str,
    expects_error: bool,
    errors: &[crate::core::ir::ErrorDef],
    documentation_snippet: bool,
) {
    // cbindgen's `[export] prefix` (shouty-snake), not a bare uppercase — see
    // `c_consumer::export_type_prefix`. ~keep
    let prefix_upper = crate::codegen::c_consumer::export_type_prefix(prefix);
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

                let json_val = crate::e2e::codegen::resolve_field(&fixture.input, &arg.field);

                if !json_val.is_null() {
                    let val = json_val;
                    let normalized = transform_json_keys_for_language(val, "snake_case");
                    let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                    let escaped = escape_c(&json_str);
                    let _ = writeln!(
                        out,
                        "    {prefix_upper}AlefHandle {var_name} = \
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
                        out.push_str(&crate::e2e::template_env::render(
                            "c/test_pass_if_null.jinja",
                            minijinja::context! { variable => var_name },
                        ));
                    } else {
                        let _ = writeln!(out, "    assert({var_name} != 0 && \"failed to build request\");");
                    }
                    request_handle_vars.push((arg.name.clone(), var_name));
                }
            }
            "string" | "mock_url" => {
                // Pass string args (e.g. file_id for file_content) directly as
                // C string literals.
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                let val = fixture.input.get(field);
                let expr = match val {
                    Some(serde_json::Value::String(s)) if arg.arg_type == "mock_url" && fixture.preserve_input_urls => {
                        format!("\"{}\"", escape_c(s))
                    }
                    _ if arg.arg_type == "mock_url" => "base_url".to_string(),
                    Some(serde_json::Value::String(s)) => format!("\"{}\"", escape_c(s)),
                    Some(serde_json::Value::Null) | None if arg.optional => "NULL".to_string(),
                    Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "NULL".to_string()),
                    None => "NULL".to_string(),
                };
                string_arg_exprs.push(expr);
            }
            "handle" => {
                // A pre-built scalar `AlefHandle` arg (not constructed via `_from_json`
                // here) — not currently exercised by byte-buffer methods, but if one
                // appears it must use the handle's `0` "none" sentinel, not `NULL`.
                string_arg_exprs.push(c_optional_sentinel("handle").to_string());
            }
            _ => {
                // Other arg types are not currently exercised by byte-buffer
                // methods; pass NULL so the call shape compiles.
                string_arg_exprs.push("NULL".to_string());
            }
        }
    }

    let fixture_id = &fixture.id;
    // ~keep A documentation snippet is published verbatim to readers, so the mock-server
    // wiring below must not reach it — mirrors `test_function::render_test_function`'s
    // `has_mock` and the `api_key` local it declares for the docs path before dispatching
    // here. Test mode passes `false` and keeps the mock branches byte-for-byte.
    let has_mock = fixture.needs_mock_server() && !documentation_snippet;
    if has_mock {
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
            "    {prefix_upper}AlefHandle client = {prefix}_{factory}(\"test-key\", base_url, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    } else if documentation_snippet {
        let _ = writeln!(
            out,
            "    {prefix_upper}AlefHandle client = {prefix}_{factory}(api_key, NULL, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    } else {
        let _ = writeln!(
            out,
            "    {prefix_upper}AlefHandle client = {prefix}_{factory}(\"test-key\", NULL, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    }
    let _ = writeln!(out, "    assert(client != 0 && \"failed to create client\");");

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
        // ~keep The failure assert and the error epilogue both run BEFORE every `_free`:
        // `{prefix}_last_error_context()` borrows thread-local storage that the next FFI call
        // clears (`catch_ffi_panic` opens with `clear_last_error()`), so a free in between would
        // leave the epilogue comparing against a wiped buffer.
        let _ = writeln!(out, "    assert(status != 0 && \"expected call to fail\");");
        super::test_function::emit_c_error_epilogue(out, prefix, fixture, errors, documentation_snippet);
        for (_, var_name) in &request_handle_vars {
            let req_snake = var_name.strip_suffix("_handle").unwrap_or(var_name);
            let _ = writeln!(out, "    {prefix}_{req_snake}_free({var_name});");
        }
        let _ = writeln!(out, "    {prefix}_default_client_free(client);");
        // free_bytes accepts a NULL ptr (no-op), so it is safe regardless of
        // whether the failed call wrote out_ptr.
        let _ = writeln!(out, "    {prefix}_free_bytes(out_ptr, out_len, out_cap);");
        let _ = writeln!(out, "}}");
        return;
    }

    // ~keep alef #243: `test_function::assemble_snippet_body` strips every `assert(` line on
    // this success path when publishing a documentation snippet, which orphaned `status` (its
    // only consumer) and failed the published example's `-Werror -Wunused-variable` build.
    // Emit the same `if (<failure>) { return EXIT_FAILURE; }` idiom that path already publishes
    // for the `expects_error` case instead of a plain `assert`, so the snippet compiles and
    // still teaches readers to check the status code rather than dropping it silently.
    if documentation_snippet {
        let _ = writeln!(out, "    if (status != 0) {{ return EXIT_FAILURE; }}");
    } else {
        let _ = writeln!(out, "    assert(status == 0 && \"expected call to succeed\");");
    }

    // Render assertions. For byte-buffer methods, the only meaningful per-field
    // assertions are presence/length checks on the buffer itself. Field names
    // (e.g. "audio", "content") are pseudo-fields — collapse them all to
    // `out_len > 0`.
    let mut emitted_len_check = false;
    for assertion in &fixture.assertions {
        // `mock.*` (alef issue #443) is not a pseudo-field of the byte buffer -- without this,
        // any assertion type other than `not_error`/`not_empty`/`not_null` (every `mock.*`
        // comparison included) fell into the catch-all "not meaningful on raw byte buffer"
        // comment below. Intercept first. ~keep
        if super::assertion_mock_capture::try_render_mock_capture_assertion(out, assertion) {
            continue;
        }
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

#[cfg(test)]
mod batch_url_regression_tests;
#[cfg(test)]
mod mock_capture_regression_tests;
#[cfg(test)]
mod namespace_strip_tests;
