use super::args::build_args_and_setup;
use super::assertions::{assertion_emits_code, render_assertion, render_json_assertion};
use super::http::render_http_test_case;
use super::visitor::{emit_visitor_test_body, resolve_zig_visitor_call_symbols};
use super::*;
use crate::core::hash::{self, CommentStyle};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    function_name: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    module_name: &str,
    ffi_prefix: &str,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let _ = writeln!(out, "const std = @import(\"std\");");
    let _ = writeln!(out, "const testing = std.testing;");
    let _ = writeln!(out, "const {module_name} = @import(\"{module_name}\");");
    let _ = writeln!(out);

    // Suppress C++ static destructors that may abort during exit (e.g., leptonica's ObjectCache cleanup).
    // The Zig test runner's --listen=- IPC protocol expects a clean exit, but C++ cleanup can trigger
    // SIGABRT. Using SIG_IGN (signal number 1) ignores SIGABRT entirely, allowing normal exit.
    let _ = writeln!(
        out,
        "// Suppress C++ global destructor aborts that break zig's --listen=- IPC"
    );
    let _ = writeln!(out, "extern \"c\" fn signal(sig: i32, handler: usize) usize;");
    let _ = writeln!(out, "var _abort_handler_installed: bool = false;");
    let _ = writeln!(out, "fn suppress_abort() void {{");
    let _ = writeln!(out, "    if (!_abort_handler_installed) {{");
    let _ = writeln!(out, "        // SIGABRT = 6 on POSIX; SIG_IGN = 1");
    let _ = writeln!(out, "        _ = signal(6, 1);");
    let _ = writeln!(out, "        _abort_handler_installed = true;");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    // Propagate the configured e2e environment to native code that reads it via getenv
    // (e.g. SSRF allow-listing for the loopback mock server). Zig has no per-suite setup
    // hook, so each test body calls allow_private_network() right after suppress_abort().
    // The managed environment does not reach libc, so push each value through setenv.
    if !e2e_config.env.is_empty() {
        let _ = writeln!(
            out,
            "extern \"c\" fn setenv(name: [*:0]const u8, value: [*:0]const u8, overwrite: c_int) c_int;"
        );
        let _ = writeln!(out, "fn allow_private_network() void {{");
        let mut keys: Vec<&String> = e2e_config.env.keys().collect();
        keys.sort();
        for k in keys {
            let v = &e2e_config.env[k];
            let _ = writeln!(out, "    _ = setenv(\"{k}\", \"{v}\", 1);");
        }
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "// E2e tests for category: {category}");
    let _ = writeln!(out);

    for fixture in fixtures {
        if fixture.http.is_some() {
            render_http_test_case(&mut out, fixture);
        } else {
            render_test_fn(
                &mut out,
                fixture,
                e2e_config,
                function_name,
                result_var,
                args,
                module_name,
                ffi_prefix,
                config,
                type_defs,
            );
        }
        let _ = writeln!(out);
    }

    out
}

#[derive(Debug, Clone)]
struct ZigStreamingAdapterMetadata {
    owner_type: String,
    item_type: String,
    request_type: String,
    adapter_name: String,
}

fn resolve_zig_streaming_adapter(
    config: &ResolvedCrateConfig,
    function_name: &str,
) -> Option<ZigStreamingAdapterMetadata> {
    config
        .adapters
        .iter()
        .find(|adapter| matches!(adapter.pattern, AdapterPattern::Streaming) && adapter.name == function_name)
        .and_then(|adapter| {
            Some(ZigStreamingAdapterMetadata {
                owner_type: adapter.owner_type.clone()?,
                item_type: adapter.item_type.clone()?,
                request_type: adapter
                    .request_type
                    .as_deref()
                    .and_then(|path| path.rsplit("::").next())
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)?,
                adapter_name: adapter.name.clone(),
            })
        })
}

#[allow(clippy::too_many_arguments)]
fn render_test_fn(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    _function_name: &str,
    _result_var: &str,
    _args: &[crate::e2e::config::ArgMapping],
    module_name: &str,
    ffi_prefix: &str,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) {
    // Resolve per-fixture call config.
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        e2e_config.effective_fields_method_calls(call_config),
    );
    let field_resolver = &call_field_resolver;
    let enum_fields = e2e_config.effective_fields_enum(call_config);
    let lang = "zig";
    let call_overrides = call_config.overrides.get(lang);
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    let result_var = &call_config.result_var;
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs);
    let args = recipe.args;
    // Client factory: when set, the test instantiates a client object via
    // `module.factory_fn(...)` and calls methods on the instance rather than
    // calling top-level package functions directly.
    // Mirrors the go codegen pattern (go.rs:981-1028 / CallOverride.client_factory).
    let client_factory = call_overrides.and_then(|o| o.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.client_factory.as_deref())
    });

    // When `result_is_json_struct = true`, the Zig function returns `[]u8` JSON.
    // The test parses it with `std.json.parseFromSlice(std.json.Value, ...)` and
    // traverses the dynamic JSON object for field assertions.
    //
    // Client-factory methods on opaque handles always return JSON `[]u8` because
    // the zig backend serializes struct results via the FFI's `*_to_json` helper
    // (see alef-backend-zig/src/gen_bindings/opaque_handles.rs). Force the flag
    // on whenever a client_factory is in play so the test path parses the JSON
    // result rather than attempting direct field access on `[]u8`.
    //
    // Exception: when the call returns raw bytes (e.g. speech/file_content use the
    // FFI byte-buffer out-pointer shape and return `[]u8` audio/file bytes rather
    // than a serialised struct). Detect this by checking the call-level flag first
    // and then falling back to any per-language override that declares `result_is_bytes`.
    // The zig and C bindings share the same byte-buffer convention, so a C override
    // of `result_is_bytes = true` is a reliable proxy when no zig override exists.
    let call_result_is_bytes = call_config.result_is_bytes || call_config.overrides.values().any(|o| o.result_is_bytes);
    let result_is_json_struct =
        !call_result_is_bytes && (call_overrides.is_some_and(|o| o.result_is_json_struct) || client_factory.is_some());

    // Whether the bare wrapper return type is `?T` (Optional). The zig backend
    // emits `?[]u8` for nullable JSON results and `?<Primitive>` for nullable
    // primitives, so assertions on the bare result must use null-checks rather
    // than `.len`.
    let result_is_option = call_overrides.is_some_and(|o| o.result_is_option) || call_config.result_is_option;

    // `result_is_simple` is a Rust-side property of the call's return type and
    // applies identically to every binding. Read it from the call-level field
    // first (preferred), and fall back to the per-call language override for
    // backwards compatibility.
    let result_is_simple = call_config.result_is_simple || call_overrides.is_some_and(|o| o.result_is_simple);

    // Whether the Zig wrapper returns an error union (`try` is required).
    //
    // The Zig backend nearly always returns an error union: any function with
    // string/path/json_object/bytes parameters must allocate a null-terminated
    // copy (→ `error{OutOfMemory}!T`), any fallible function (`returns_result`)
    // wraps a `DomainError||error{OutOfMemory}!T`, and any function whose return
    // type is a string/JSON/collection blob also needs heap allocation.
    //
    // The ONLY case where `try` is incorrect is a function that is:
    //   - genuinely infallible (no Rust Result<T,E>)
    //   - takes no allocating parameters (no string/path/bytes/json_object args)
    //   - returns a primitive directly (u64, bool, etc.)
    //
    // Rather than attempting to infer this from incomplete config information,
    // we default to emitting `try` and require an explicit opt-out:
    //
    //   [crates.e2e.calls.language_count.overrides.zig]
    //   returns_result = false
    //
    // Special case: functions named `unregister_*` always return error unions
    // (plugin trait unregister calls) and must always use `try`, regardless
    // of the `returns_result` override.
    //
    // This is safer than guessing wrong and producing un-compilable Zig.
    let call_returns_error_union =
        function_name.starts_with("unregister_") || call_overrides.and_then(|o| o.returns_result) != Some(false);

    let test_name = fixture.id.to_snake_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let (setup_lines, args_str, setup_needs_gpa) = build_args_and_setup(
        &fixture.input,
        args,
        &fixture.id,
        module_name,
        config,
        type_defs,
        fixture,
    );
    // Append per-call zig extra_args (e.g. `["null"]` for the trailing
    // optional `query` parameter on `list_files` / `list_batches`). Mirrors
    // the same mechanism used by go/python/swift codegen — zig's method
    // signatures require every optional positional argument to be supplied
    // explicitly, so the e2e config carries a per-language extras list.
    let extra_args = recipe.extra_args;
    let args_str = if extra_args.is_empty() {
        args_str
    } else if args_str.is_empty() {
        extra_args.join(", ")
    } else {
        format!("{args_str}, {}", extra_args.join(", "))
    };

    // Pre-compute whether any assertion will emit code that references `result` /
    // `allocator`. Used to decide whether to emit the GPA allocator binding.
    let any_happy_emits_code = fixture
        .assertions
        .iter()
        .any(|a| assertion_emits_code(a, field_resolver));
    let any_non_error_emits_code = fixture
        .assertions
        .iter()
        .filter(|a| a.assertion_type != "error")
        .any(|a| assertion_emits_code(a, field_resolver));

    // Pre-compute streaming-virtual path conditions.
    let has_streaming_virtual_assertions = fixture.assertions.iter().any(|a| {
        a.field
            .as_ref()
            .is_some_and(|f| !f.is_empty() && is_streaming_virtual_field(f))
    });
    let is_stream_fn = function_name.contains("stream");
    let streaming_adapter = if has_streaming_virtual_assertions && is_stream_fn && client_factory.is_some() {
        resolve_zig_streaming_adapter(config, &function_name)
    } else {
        None
    };
    let uses_streaming_virtual_path =
        result_is_json_struct && has_streaming_virtual_assertions && is_stream_fn && client_factory.is_some();
    // Whether the streaming-virtual path also parses JSON (for non-streaming assertions).
    let streaming_path_has_non_streaming = uses_streaming_virtual_path
        && fixture.assertions.iter().any(|a| {
            !a.field
                .as_ref()
                .is_some_and(|f| !f.is_empty() && is_streaming_virtual_field(f))
                && !matches!(a.assertion_type.as_str(), "not_error" | "error")
                && a.field
                    .as_ref()
                    .is_some_and(|f| !f.is_empty() && field_resolver.is_valid_for_result(f))
        });

    let _ = writeln!(out, "test \"{test_name}\" {{");
    let _ = writeln!(out, "    // {description}");
    let _ = writeln!(out, "    suppress_abort();");
    if !e2e_config.env.is_empty() {
        let _ = writeln!(out, "    allow_private_network();");
    }

    // Visitor fixtures bypass the high-level `convert(html, options)` wrapper
    // and inline the FFI sequence so we can attach the generated visitor callbacks
    // vtable to the options handle. The vtable is populated by per-fixture
    // C-callable thunks emitted by `zig_visitors::build_zig_visitor`.
    if let Some(visitor_spec) = &fixture.visitor {
        let html = fixture.input.get("html").and_then(|v| v.as_str()).unwrap_or_default();
        let options_value = fixture.input.get("options").cloned();
        let visitor_symbols = resolve_zig_visitor_call_symbols(call_config, &recipe, ffi_prefix);
        emit_visitor_test_body(
            out,
            &fixture.id,
            html,
            options_value.as_ref(),
            visitor_spec,
            module_name,
            &visitor_symbols,
            &fixture.assertions,
            expects_error,
            field_resolver,
        );
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
        return;
    }

    // Emit GPA allocator only when it will actually be used: setup lines that
    // need GPA allocation (mock_url), or a JSON-struct result path where the test
    // will call `std.json.parseFromSlice`. The binding is not needed for
    // error-only paths or tests with no field assertions.
    // Note: `bytes` arg setup uses c_allocator directly and does NOT require GPA.
    // For the streaming-virtual path, `allocator` is only needed if there are also
    // non-streaming assertions that require JSON parsing via parseFromSlice.
    let needs_gpa = setup_needs_gpa
        || streaming_path_has_non_streaming
        || (!uses_streaming_virtual_path && result_is_json_struct && !expects_error && any_happy_emits_code)
        || (!uses_streaming_virtual_path && result_is_json_struct && expects_error && any_non_error_emits_code);
    if needs_gpa {
        let _ = writeln!(out, "    var gpa: std.heap.DebugAllocator(.{{}}) = .init;");
        let _ = writeln!(out, "    defer _ = gpa.deinit();");
        let _ = writeln!(out, "    const allocator = gpa.allocator();");
        let _ = writeln!(out);
    }

    for line in &setup_lines {
        let _ = writeln!(out, "    {line}");
    }

    // Client factory: when configured, instantiate a client object via the named
    // constructor function and call the method on the instance.
    // The client is pointed at MOCK_SERVER_URL/fixtures/<id> (mirrors go.rs:981-1028).
    // When not configured, fall back to calling the top-level package function directly.
    let call_prefix = if let Some(factory) = client_factory {
        let fixture_id = &fixture.id;
        let _ = writeln!(
            out,
            "    const _mock_url = try std.fmt.allocPrintSentinel(std.heap.c_allocator, \"{{s}}/fixtures/{fixture_id}\", .{{if (std.c.getenv(\"MOCK_SERVER_URL\")) |v| std.mem.span(v) else \"http://localhost:8080\"}}, 0);"
        );
        let _ = writeln!(out, "    defer std.heap.c_allocator.free(_mock_url);");
        let _ = writeln!(
            out,
            "    var _client = try {module_name}.{factory}(\"test-key\", _mock_url, null, null, null);"
        );
        let _ = writeln!(out, "    defer _client.free();");
        "_client".to_string()
    } else {
        module_name.to_string()
    };

    if expects_error {
        // Error-path test: use error union syntax `!T` and try-catch.
        // Async functions execute via tokio::runtime::block_on in the FFI shim,
        // so the call site is synchronous from Zig's perspective.
        if result_is_json_struct {
            let _ = writeln!(
                out,
                "    const _result_json = {call_prefix}.{function_name}({args_str}) catch {{"
            );
        } else {
            let _ = writeln!(
                out,
                "    const result = {call_prefix}.{function_name}({args_str}) catch {{"
            );
        }
        let _ = writeln!(out, "        try testing.expect(true); // Error occurred as expected");
        let _ = writeln!(out, "        return;");
        let _ = writeln!(out, "    }};");
        // Whether any non-error assertion will emit code that references `result`.
        // If not, we must explicitly discard `result` to satisfy Zig's
        // strict-unused-locals rule.
        let any_emits_code = fixture
            .assertions
            .iter()
            .filter(|a| a.assertion_type != "error")
            .any(|a| assertion_emits_code(a, field_resolver));
        if result_is_json_struct && any_emits_code {
            let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
            let _ = writeln!(
                out,
                "    var _parsed = try std.json.parseFromSlice(std.json.Value, allocator, std.mem.span(_result_json), .{{}});"
            );
            let _ = writeln!(out, "    defer _parsed.deinit();");
            let _ = writeln!(out, "    const {result_var} = &_parsed.value;");
            let _ = writeln!(out, "    // Perform success assertions if any");
            for assertion in &fixture.assertions {
                if assertion.assertion_type != "error" {
                    render_json_assertion(out, assertion, result_var, field_resolver, false);
                }
            }
        } else if result_is_json_struct {
            let _ = writeln!(out, "    _ = _result_json;");
        } else if any_emits_code {
            let _ = writeln!(out, "    // Perform success assertions if any");
            for assertion in &fixture.assertions {
                if assertion.assertion_type != "error" {
                    render_assertion(
                        out,
                        assertion,
                        result_var,
                        field_resolver,
                        enum_fields,
                        result_is_option,
                        result_is_simple,
                    );
                }
            }
        } else {
            let _ = writeln!(out, "    _ = result;");
        }
    } else if fixture.assertions.is_empty() {
        // No assertions: emit a call to verify compilation.
        if result_is_json_struct {
            let _ = writeln!(
                out,
                "    const _result_json = try {call_prefix}.{function_name}({args_str});"
            );
            let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
        } else if call_returns_error_union {
            let _ = writeln!(out, "    _ = try {call_prefix}.{function_name}({args_str});");
        } else {
            let _ = writeln!(out, "    _ = {call_prefix}.{function_name}({args_str});");
        }
    } else {
        // Happy path: call and assert. Detect whether any assertion actually
        // emits code that references `result` (some — like `not_error` — emit
        // nothing) so we don't leave an unused local, which Zig 0.16 rejects.
        let any_emits_code = fixture
            .assertions
            .iter()
            .any(|a| assertion_emits_code(a, field_resolver));
        if call_result_is_bytes && client_factory.is_some() {
            // Bytes path: the function returns raw `[]u8` (audio/file bytes), not
            // a JSON struct. Call, defer-free, then check len for not_empty/is_empty.
            let _ = writeln!(
                out,
                "    const _result_json = try {call_prefix}.{function_name}({args_str});"
            );
            let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
            let has_bytes_assertions = fixture
                .assertions
                .iter()
                .any(|a| matches!(a.assertion_type.as_str(), "not_empty" | "is_empty"));
            if has_bytes_assertions {
                for assertion in &fixture.assertions {
                    match assertion.assertion_type.as_str() {
                        "not_empty" => {
                            let _ = writeln!(out, "    try testing.expect(std.mem.span(_result_json).len > 0);");
                        }
                        "is_empty" => {
                            let _ = writeln!(
                                out,
                                "    try testing.expectEqual(@as(usize, 0), std.mem.span(_result_json).len);"
                            );
                        }
                        "not_error" | "error" => {}
                        _ => {
                            let atype = &assertion.assertion_type;
                            let _ = writeln!(
                                out,
                                "    // bytes result: assertion '{atype}' not implemented for zig bytes"
                            );
                        }
                    }
                }
            }
        } else if result_is_json_struct {
            // When streaming-virtual field assertions are present (pre-computed above),
            // emit raw FFI code to collect all chunks instead of calling
            // the high-level streaming wrapper (which only returns the last chunk's JSON).
            if uses_streaming_virtual_path {
                let Some(streaming_adapter) = streaming_adapter.as_ref() else {
                    let _ = writeln!(
                        out,
                        "    // skipped: streaming fixture requires matching [[crates.adapters]] metadata for zig e2e codegen"
                    );
                    let _ = writeln!(out, "    return error.SkipZigTest;");
                    let _ = writeln!(out, "}}");
                    let _ = writeln!(out);
                    return;
                };
                let owner_snake = streaming_adapter.owner_type.to_snake_case();
                let request_snake = streaming_adapter.request_type.to_snake_case();
                let request_from_json = format!("{ffi_prefix}_{request_snake}_from_json");
                let request_free = format!("{ffi_prefix}_{request_snake}_free");
                let stream_start = format!("{ffi_prefix}_{owner_snake}_{}_start", streaming_adapter.adapter_name);
                let stream_free = format!("{ffi_prefix}_{owner_snake}_{}_free", streaming_adapter.adapter_name);
                let client_c_type = format!("{}{}", ffi_prefix.to_shouty_snake_case(), streaming_adapter.owner_type);

                // Streaming-virtual path: inline FFI collect.
                // Build a sentinel-terminated request string.
                let _ = writeln!(
                    out,
                    "    const _req_z = try std.heap.c_allocator.dupeZ(u8, {args_str});"
                );
                let _ = writeln!(out, "    defer std.heap.c_allocator.free(_req_z);");
                let _ = writeln!(
                    out,
                    "    const _req_handle = {module_name}.c.{request_from_json}(_req_z.ptr);"
                );
                let _ = writeln!(out, "    defer {module_name}.c.{request_free}(_req_handle);");
                let _ = writeln!(
                    out,
                    "    const _stream_handle = {module_name}.c.{stream_start}(@as(*{module_name}.c.{client_c_type}, @ptrCast(_client._handle)), _req_handle);"
                );
                let _ = writeln!(out, "    if (_stream_handle == null) return error.StreamStartFailed;");
                let _ = writeln!(out, "    defer {module_name}.c.{stream_free}(_stream_handle);");
                // Emit the collect snippet (already has 4-space indentation baked in).
                let snip = StreamingFieldResolver::collect_snippet_zig(
                    "_stream_handle",
                    "chunks",
                    module_name,
                    ffi_prefix,
                    &streaming_adapter.owner_type,
                    &streaming_adapter.adapter_name,
                    &streaming_adapter.item_type,
                );
                out.push_str("    ");
                out.push_str(&snip);
                out.push('\n');
                // For non-streaming assertions (e.g. usage), we also need _result_json.
                // Re-serialize the last chunk in `chunks` to get the JSON.
                if streaming_path_has_non_streaming {
                    let _ = writeln!(
                        out,
                        "    const _result_json = if (chunks.items.len > 0) chunks.items[chunks.items.len - 1] else &[_]u8{{}};"
                    );
                    let _ = writeln!(
                        out,
                        "    var _parsed = try std.json.parseFromSlice(std.json.Value, allocator, _result_json, .{{}});"
                    );
                    let _ = writeln!(out, "    defer _parsed.deinit();");
                    let _ = writeln!(out, "    const {result_var} = &_parsed.value;");
                }
                for assertion in &fixture.assertions {
                    render_json_assertion(out, assertion, result_var, field_resolver, true);
                }
            } else {
                // JSON struct path: parse result JSON and access fields dynamically.
                let _ = writeln!(
                    out,
                    "    const _result_json = try {call_prefix}.{function_name}({args_str});"
                );
                let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
                if any_emits_code {
                    // For certain functions like `interact()`, the result is a struct that
                    // the fixture expects to access via a wrapper field (e.g. "interaction.action_results").
                    // Since the Zig binding returns the serialized struct directly (without wrapping),
                    // we wrap it in a JSON object with the appropriate key before parsing.
                    let wrap_field = match function_name.as_str() {
                        "interact" => Some("interaction"),
                        _ => None,
                    };

                    let parse_json_var = if let Some(field) = wrap_field {
                        // Build the Zig format string for wrapping: {"field":{s}}
                        // In Zig: `std.fmt.allocPrint(..., "{\"field\":{s}}", .{value})`
                        // In Rust string literal: "{{{{\\\"field\\\":{{s}}}}}}" (each { → {{, each \ → \\)
                        let _ = writeln!(
                            out,
                            "    const _wrapped_json = try std.fmt.allocPrint(allocator, \"{{{{\\\"{}\\\":{{s}}}}}}\", .{{std.mem.span(_result_json)}});",
                            field
                        );
                        let _ = writeln!(out, "    defer allocator.free(_wrapped_json);");
                        "_wrapped_json".to_string()
                    } else {
                        // C string pointers require std.mem.span() conversion to [](const u8.
                        "std.mem.span(_result_json)".to_string()
                    };

                    let _ = writeln!(
                        out,
                        "    var _parsed = try std.json.parseFromSlice(std.json.Value, allocator, {parse_json_var}, .{{}});"
                    );
                    let _ = writeln!(out, "    defer _parsed.deinit();");
                    let _ = writeln!(out, "    const {result_var} = &_parsed.value;");
                    for assertion in &fixture.assertions {
                        render_json_assertion(out, assertion, result_var, field_resolver, false);
                    }
                }
            }
        } else if any_emits_code {
            let try_kw = if call_returns_error_union { "try " } else { "" };
            let _ = writeln!(
                out,
                "    const {result_var} = {try_kw}{call_prefix}.{function_name}({args_str});"
            );
            for assertion in &fixture.assertions {
                render_assertion(
                    out,
                    assertion,
                    result_var,
                    field_resolver,
                    enum_fields,
                    result_is_option,
                    result_is_simple,
                );
            }
        } else if call_returns_error_union {
            let _ = writeln!(out, "    _ = try {call_prefix}.{function_name}({args_str});");
        } else {
            let _ = writeln!(out, "    _ = {call_prefix}.{function_name}({args_str});");
        }
    }

    let _ = writeln!(out, "}}");
}
