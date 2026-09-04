use super::args::build_args_and_setup;
use super::assertions::{assertion_emits_code, render_assertion, render_json_assertion};
use super::http::render_http_test_case;
use super::stubs::test_backend_out_err_var_name;
use super::visitor::{emit_visitor_test_body, resolve_zig_visitor_call_symbols};
use super::*;
use crate::core::hash::{self, CommentStyle};

/// Emit a call whose Zig wrapper reports success via an `i32` return code plus an
/// `out_error` pointer — the trait-bridge `register_*` shape — instead of a Zig
/// error union. `_ = call(...)` alone discards both the return code and the
/// pointer, so a registration that fails (bad vtable, duplicate name, a backend
/// that fails its own validation) is indistinguishable from one that succeeds.
/// This checks the return code, surfaces the `out_error` message naming the
/// failing call, and frees the message on the failure path so it cannot leak. ~keep
fn emit_test_backend_register_call(
    out: &mut String,
    call_prefix: &str,
    function_name: &str,
    args_str: &str,
    out_err_var: &str,
) {
    let _ = writeln!(out, "    const _rc = {call_prefix}.{function_name}({args_str});");
    let _ = writeln!(out, "    if (_rc != 0) {{");
    let _ = writeln!(
        out,
        "        const _msg = if ({out_err_var}) |_m| std.mem.span(_m) else \"unknown error\";"
    );
    let _ = writeln!(
        out,
        "        std.debug.print(\"{function_name} failed: {{s}}\\n\", .{{_msg}});"
    );
    let _ = writeln!(out, "        if ({out_err_var}) |_m| {call_prefix}._free_string(_m);");
    let _ = writeln!(out, "        return error.TestUnexpectedResult;");
    let _ = writeln!(out, "    }}");
}

/// Close the error arm of the `if (call) |_| {..} else |_| {..}` shape and, when the fixture
/// declares an `error` value this backend can substantiate, actually compare it — or, when the
/// value names a real variant Zig cannot substantiate, emit the registered skip instead of an
/// assertion that can never pass.
///
/// ~keep Zig sits on the same C ABI as the `c` backend, which reports a failure as
/// `set_last_error(alef_ffi_error_code(&e), &e.to_string())` — a Display message plus a numeric
/// taxonomy code. The generated binding exposes the message as `_last_error()` and dispatches the
/// code to a declared error-set member ONLY for variants that declared `#[alef(error_code = N)]`
/// (`declared_error_variant::classify` decides this once); an uncoded variant collapses to
/// `error.UnknownFfiError`, which `@errorName` can never match against a real variant's name.
/// Before this, the declared value was discarded and every valued `error` assertion was
/// weakened to a bare "the call failed" check that could not tell one failure from another.
///
/// ~keep The unsubstantiable arm still captures the error, and captures rather than discards it:
/// the variant's *identity* is out of reach, but the ABI's two failure-reporting channels are
/// not. `set_last_error(alef_ffi_error_code(&e), &e.to_string())` fills both on every failure
/// path, so a binding that reports neither — empty message AND the catch-all error-set member —
/// is a real regression this arm now fails on. `_ = _err;` made it pass unconditionally.
fn emit_declared_error_value_assertion(
    out: &mut String,
    fixture: &Fixture,
    errors: &[crate::core::ir::ErrorDef],
    module_name: &str,
    for_docs: bool,
) {
    // ~keep A documentation snippet is a `main`, not a `test`: `testing` is never bound in
    // `zig/snippet_body.jinja`, so `try testing.expect(..)` would not compile there, and the
    // snippet emitter rewrites the literal `else |_| {}` arm into a printing one. Snippet output
    // therefore stays byte-identical to before this change.
    if for_docs {
        out.push_str(&render_declared_error_branch("bare", module_name, "", ""));
        return;
    }
    use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, skip_line};
    match classify("zig", fixture, errors) {
        DeclaredErrorAssertion::Undeclared => {
            out.push_str(&render_declared_error_branch("bare", module_name, "", ""));
        }
        DeclaredErrorAssertion::Assert(declared) => {
            out.push_str(&render_declared_error_branch(
                "assert",
                module_name,
                &escape_zig(declared),
                "",
            ));
        }
        DeclaredErrorAssertion::Unsubstantiable(variant) => {
            let skip = skip_line("        ", "//", variant, &fixture.id, "zig");
            out.push_str(&render_declared_error_branch("unsubstantiable", module_name, "", &skip));
        }
    }
}

/// The error-set member `backends::zig::gen_bindings::helpers::gen_last_error_helpers` collapses
/// every uncoded variant — and every code matching no declared variant — onto. Named here rather
/// than spelled inline so the generated assertion and the generator that produces the value it
/// tests against move together. ~keep
const ZIG_UNKNOWN_ERROR_NAME: &str = "UnknownFfiError";

fn render_declared_error_branch(kind: &str, module_name: &str, expected: &str, skip_line: &str) -> String {
    crate::e2e::template_env::render(
        "zig/declared_error_branch.jinja",
        minijinja::context! {
            kind => kind,
            module_name => module_name,
            expected => expected,
            skip_line => skip_line,
            unknown_error_name => ZIG_UNKNOWN_ERROR_NAME,
        },
    )
}

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
    errors: &[crate::core::ir::ErrorDef],
    ir: crate::e2e::codegen::call_ir::CallIr<'_>,
    enums: &[crate::core::ir::EnumDef],
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let _ = writeln!(out, "const std = @import(\"std\");");
    let _ = writeln!(out, "const testing = std.testing;");
    let _ = writeln!(out, "const {module_name} = @import(\"{module_name}\");");
    let _ = writeln!(out);

    // Propagate the configured e2e environment to native code that reads it via getenv. Zig has no per-suite setup
    // hook, so each test body calls allow_private_network(). The managed environment does not reach libc, so push each
    // value through setenv. ~keep
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
                errors,
                true,
                false,
                ir,
                enums,
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
    errors: &[crate::core::ir::ErrorDef],
    wrap_as_test: bool,
    for_docs: bool,
    ir: crate::e2e::codegen::call_ir::CallIr<'_>,
    enums: &[crate::core::ir::EnumDef],
) {
    // Resolve per-fixture call config.
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    let lang = "zig";
    let call_overrides = call_config.overrides.get(lang);
    // `field_resolver.is_enum` consults `effective_enum_fields` (hand-maintained config) first
    // and only then the IR-derived classification (`with_ir_enum_map`), so an explicit config
    // entry still wins. Mirrors the gleam e2e generator's fix for the same defect: a config-only
    // check answered `false` for every enum-typed field a consumer's `alef.toml` never listed,
    // so a typed-struct `equals` assertion on it compared a Zig enum value against a raw
    // `[]const u8` literal via `testing.expectEqual` — a type mismatch `zig build` rejects. ~keep
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        e2e_config.effective_fields_method_calls(call_config),
    )
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    .with_wire_optional_fields(FieldResolver::ir_wire_optional_fields(type_defs))
    .with_enum_fields(super::enum_field_config::effective_enum_fields(
        e2e_config,
        call_config,
        call_overrides,
    ))
    .with_ir_enum_map(
        FieldResolver::ir_enum_fields(type_defs, enums),
        crate::e2e::codegen::call_ir::resolve_declared_result_type(call_config, lang, ir),
    )
    // `with_ir_fields` only proves a bare field name optional, with no path context; anchors
    // this fixture's assertion paths via the IR's real per-type walk instead, matching
    // `presentation.rs`'s existing `with_anchored_optional_paths` use. ~keep
    .with_anchored_optional_paths(fixture.assertions.iter().filter_map(|a| a.field.as_deref()));
    let field_resolver = &call_field_resolver;
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    let result_var = call_config.effective_result_var();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs);
    let args = recipe.args;
    // A `test_backend` arg means `emit_test_backend` (stubs.rs) declared an `out_error`
    // pointer for this call. Combined with `call_returns_error_union == false` below, that
    // identifies the register-fn shape: an `i32` return code plus an out-param, not a Zig
    // error union `try` can unwrap. `None` when no such arg is present, so the ordinary
    // `_ = call(...)` / `try` paths are unaffected. ~keep
    let register_out_err_var = args
        .iter()
        .any(|arg| arg.arg_type == "test_backend")
        .then(|| test_backend_out_err_var_name(&fixture.id));
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
    // The IR fallback closes the gap `zig_return_type` opens: it maps EVERY `Named` struct
    // return with `has_serde` to `[]u8` unconditionally, whether or not the e2e call config
    // ever declared `result_is_json_struct` or a `client_factory`. Additive only — `||`'d onto
    // the existing config-driven checks, so an explicit `false` cannot be produced here, only a
    // `true` a config author never had to spell out. See `result_shape::ir_says_json_struct`.
    let result_is_json_struct = !call_result_is_bytes
        && (call_overrides.is_some_and(|o| o.result_is_json_struct)
            || client_factory.is_some()
            || super::result_shape::ir_says_json_struct(call_config, lang, ir, type_defs));

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

    if wrap_as_test {
        let _ = writeln!(out, "test \"{test_name}\" {{");
        let _ = writeln!(out, "    // {description}");
        if !e2e_config.env.is_empty() {
            let _ = writeln!(out, "    allow_private_network();");
        }
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
            wrap_as_test,
        );
        if wrap_as_test {
            let _ = writeln!(out, "}}");
            let _ = writeln!(out);
        }
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
        || (!uses_streaming_virtual_path && result_is_json_struct && !expects_error && any_happy_emits_code);
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
    // In test mode the client is pointed at MOCK_SERVER_URL/fixtures/<id> (mirrors
    // go.rs:981-1028). A documentation snippet is published verbatim to readers, so it
    // must never carry that harness wiring or a literal credential: it reads the real
    // credential from the environment and leaves `base_url` at the binding default,
    // matching what `java/snippet_body.jinja` and `csharp/snippet_body.jinja` emit. ~keep
    // When not configured, fall back to calling the top-level package function directly.
    let call_prefix = if let Some(factory) = client_factory {
        if for_docs {
            let api_key_var = crate::e2e::fixture::FixtureEnv::api_key_var_or_default(fixture.env.as_ref());
            let _ = writeln!(
                out,
                "    const _api_key = std.c.getenv(\"{api_key_var}\") orelse return error.MissingApiKey;"
            );
            // The factory's second positional slot is base_url (matches the mock-server
            // arm below, which passes `_mock_url` there). A docs fixture that names
            // `docs.client.base_url` fills the slot; otherwise it stays `null` so the
            // binding falls back to its own default endpoint. ~keep
            let base_url_arg = match crate::e2e::codegen::client_factory::docs_base_url(fixture.docs_client()) {
                Some(url) => format!("\"{}\"", escape_zig(url)),
                None => "null".to_string(),
            };
            let _ = writeln!(
                out,
                "    var _client = try {module_name}.{factory}(std.mem.span(_api_key), {base_url_arg}, null, null, null);"
            );
        } else {
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
        }
        let _ = writeln!(out, "    defer _client.free();");
        "_client".to_string()
    } else {
        module_name.to_string()
    };

    if expects_error {
        // Error-path test: the call must fail. `if (expr) |_| {...} else |_| {...}`
        // captures both arms of the error union explicitly, so an unexpected success
        // fails the test via `return error.TestUnexpectedResult`. The previous shape —
        // `... catch { try testing.expect(true); return; }` — could never fail: on
        // success the catch block simply never ran and execution fell through, and
        // `expect(true)` inside the catch was a tautology on error. ~keep
        //
        // The success arm discards its capture (`|_|`) rather than binding `result`,
        // since a fixture asserting `error` has nothing meaningful to check once the
        // call has already failed the test by succeeding.
        let _ = writeln!(out, "    if ({call_prefix}.{function_name}({args_str})) |_| {{");
        let _ = writeln!(out, "        return error.TestUnexpectedResult;");
        emit_declared_error_value_assertion(out, fixture, errors, module_name, for_docs);
        if !for_docs {
            crate::e2e::codegen::error_path_assertions::emit(out, fixture, "    // ", "zig");
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
        } else if let Some(out_err_var) = register_out_err_var.as_deref() {
            emit_test_backend_register_call(out, &call_prefix, &function_name, &args_str, out_err_var);
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
                            let _ = writeln!(out, "    try testing.expect(_result_json.len > 0);");
                        }
                        "is_empty" => {
                            let _ = writeln!(out, "    try testing.expectEqual(@as(usize, 0), _result_json.len);");
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
                    "    const _stream_handle = {module_name}.c.{stream_start}(_client._handle, _req_handle);"
                );
                let _ = writeln!(out, "    if (_stream_handle == 0) return error.StreamStartFailed;");
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
                let mut assertions_body = String::new();
                for assertion in &fixture.assertions {
                    render_json_assertion(&mut assertions_body, assertion, result_var, field_resolver, true);
                }
                crate::e2e::codegen::fail_on_unavailable_field_markers(
                    &assertions_body,
                    "zig",
                    &fixture.id,
                    &fixture.assertions,
                );
                crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&assertions_body, "zig", &fixture.id);
                out.push_str(&assertions_body);
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
                            "    const _wrapped_json = try std.fmt.allocPrint(allocator, \"{{{{\\\"{}\\\":{{s}}}}}}\", .{{_result_json}});",
                            field
                        );
                        let _ = writeln!(out, "    defer allocator.free(_wrapped_json);");
                        "_wrapped_json".to_string()
                    } else {
                        // _result_json is already a []u8 slice from the Zig wrapper function,
                        // so pass it directly to parseFromSlice.
                        "_result_json".to_string()
                    };

                    let _ = writeln!(
                        out,
                        "    var _parsed = try std.json.parseFromSlice(std.json.Value, allocator, {parse_json_var}, .{{}});"
                    );
                    let _ = writeln!(out, "    defer _parsed.deinit();");
                    let _ = writeln!(out, "    const {result_var} = &_parsed.value;");
                    let mut assertions_body = String::new();
                    for assertion in &fixture.assertions {
                        render_json_assertion(&mut assertions_body, assertion, result_var, field_resolver, false);
                    }
                    crate::e2e::codegen::fail_on_unavailable_field_markers(
                        &assertions_body,
                        "zig",
                        &fixture.id,
                        &fixture.assertions,
                    );
                    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(
                        &assertions_body,
                        "zig",
                        &fixture.id,
                    );
                    out.push_str(&assertions_body);
                } else {
                    let _ = writeln!(out, "    std.debug.print(\"{{s}}\\n\", .{{_result_json}});");
                }
            }
        } else if any_emits_code {
            let try_kw = if call_returns_error_union { "try " } else { "" };
            let _ = writeln!(
                out,
                "    const {result_var} = {try_kw}{call_prefix}.{function_name}({args_str});"
            );
            let mut assertions_body = String::new();
            for assertion in &fixture.assertions {
                render_assertion(
                    &mut assertions_body,
                    assertion,
                    result_var,
                    field_resolver,
                    result_is_option,
                    result_is_simple,
                );
            }
            crate::e2e::codegen::fail_on_unavailable_field_markers(
                &assertions_body,
                "zig",
                &fixture.id,
                &fixture.assertions,
            );
            crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&assertions_body, "zig", &fixture.id);
            out.push_str(&assertions_body);
        } else if call_returns_error_union {
            let _ = writeln!(out, "    _ = try {call_prefix}.{function_name}({args_str});");
        } else if let Some(out_err_var) = register_out_err_var.as_deref() {
            emit_test_backend_register_call(out, &call_prefix, &function_name, &args_str, out_err_var);
        } else {
            let _ = writeln!(out, "    _ = {call_prefix}.{function_name}({args_str});");
        }
    }

    if wrap_as_test {
        let _ = writeln!(out, "}}");
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    module_name: &str,
    ffi_prefix: &str,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> anyhow::Result<String> {
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let result_var = call.effective_result_var();
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    if call.args.iter().any(|argument| argument.arg_type == "test_backend") {
        anyhow::bail!("zig snippet `{}` requires test-backend lifecycle teardown", fixture.id);
    }
    let mut call_fixture = fixture.clone();
    if !expects_error {
        call_fixture.assertions.clear();
    }
    let mut test = String::new();
    render_test_fn(
        &mut test,
        &call_fixture,
        e2e_config,
        "",
        "",
        &[],
        module_name,
        ffi_prefix,
        config,
        type_defs,
        &[],
        false,
        true,
        // Snippet rendering has no free-function IR to consult — only `type_defs`. This still
        // lets `ir_says_json_struct` resolve through a method on an IR type (client_factory
        // calls), the same degraded-but-not-absent state `CallIr::is_absent` already models.
        crate::e2e::codegen::call_ir::CallIr {
            functions: &[],
            type_defs,
        },
        &[],
    );
    // The test-mode error path captures the failure with a discarded `else |_|` arm
    // (nothing to report inside `test { ... }`). The snippet is a runnable `main`,
    // so swap in a named capture that prints the caught error instead. ~keep
    // Rebinding the discarded call result is a ONE-LINE, ONCE-PER-BODY edit, and both halves of
    // that had to be stated explicitly. Applied per line with no guard, `"_ = "` also matched the
    // allocator teardown every snippet emits -- `defer _ = gpa.deinit();` became
    // `defer const result = gpa.deinit();`, which is not Zig at all and failed 54 of one consumer's
    // snippets on `expected block or expression`. Requiring the discard to open the statement is
    // what separates the call from `defer`/`errdefer`-prefixed ones; `bound` stops a second,
    // later discard from being rebound to the same name. ~keep
    let mut bound = false;
    let mut body = test
        .lines()
        .map(|line| {
            let line = line.replace(
                "else |_| {}",
                "else |err| { std.debug.print(\"call failed as expected: {s}\\n\", .{@errorName(err)}); }",
            );
            if expects_error || call.returns_void || bound {
                return line;
            }
            let Some(discarded) = discarded_call_statement(&line) else {
                return line;
            };
            bound = true;
            let indent = &line[..line.len() - line.trim_start().len()];
            format!("{indent}const {result_var} = {discarded}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    // A `result_is_json_struct` call binds `_result_json` — a `[]u8` payload — instead of a
    // typed `result`, so no `docs.shows` field path has a struct to read from. Those
    // fixtures keep the whole-payload print below. ~keep
    let binds_typed_result = !expects_error && !call.returns_void && !body.contains("const _result_json =");
    let presentation = if binds_typed_result {
        crate::e2e::codegen::presentation::resolve(fixture, e2e_config, "zig", type_defs, enums, functions)
    } else {
        Vec::new()
    };
    if !expects_error && !call.returns_void && presentation.is_empty() {
        let displayed_result = if body.contains("const _result_json =") {
            "_result_json"
        } else {
            result_var
        };
        let format = if displayed_result == "_result_json" {
            "{s}"
        } else {
            "{any}"
        };
        // `join("\n")` above drops the trailing newline `render_test_fn` wrote, so appending
        // without restoring it splices the print onto the end of the last statement line —
        // legal Zig, but a published snippet a reader has to un-run-on. ~keep
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&format!(
            "    std.debug.print(\"{format}\\n\", .{{{displayed_result}}});\n"
        ));
    }
    Ok(crate::e2e::template_env::render(
        "zig/snippet_body.jinja",
        minijinja::context! { module => module_name, body => body, body_is_indented => true,
        presentation => presentation },
    ))
}

#[cfg(test)]
#[path = "snippet_tests.rs"]
mod snippet_tests;

#[cfg(test)]
#[path = "expects_error_fails_on_unexpected_success_tests.rs"]
mod expects_error_fails_on_unexpected_success_tests;

#[cfg(test)]
#[path = "error_value_and_error_field_tests.rs"]
mod error_value_and_error_field_tests;

/// The remainder of a statement that opens by discarding a CALL's return value, i.e.
/// `_ = <call>(...);`. Returns `None` for a discard that opens the statement but is not a call.
///
/// Anchored at the start of the trimmed statement on purpose: `defer _ = gpa.deinit();` and
/// `errdefer _ = ...` are discards too, and rebinding one produces `defer const x = ...`, which no
/// Zig grammar accepts. ~keep
///
/// A statement-opening discard is also not always a call: every generated visitor callback
/// unconditionally discards its unused `_ctx`, `_user_data`, and other typed parameters with
/// `_ = _ctx;` / `_ = out_custom;`, and those lines appear before any real call in a visitor
/// snippet's body. Rebinding the first one turns `_ = _ctx;` into `const result = _ctx;` — a
/// bound value that nothing reads, which Zig 0.16 rejects as an unused local constant. A call
/// discard is syntactically distinct from a bare-identifier discard: it always carries a
/// parenthesised argument list, so requiring `(...)` before the closing `;` tells the two apart.
/// ~keep
fn discarded_call_statement(line: &str) -> Option<&str> {
    let discarded = line.trim_start().strip_prefix("_ = ")?;
    let is_call = discarded.contains('(') && discarded.trim_end().ends_with(");");
    is_call.then_some(discarded)
}

#[cfg(test)]
#[path = "discard_rebinding_tests.rs"]
mod discard_rebinding_tests;

#[cfg(test)]
#[path = "register_call_check_tests.rs"]
mod register_call_check_tests;
