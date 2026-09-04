use crate::codegen::keywords::swift_ident;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::codegen::inert_example::{self, InertCause, InertExample};
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::{FieldResolver, SwiftFirstClassMap};
use crate::e2e::fixture::Fixture;
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use super::args::build_args_and_setup;
use super::assertions::render_assertion;
use super::empty_field_accessor_map;
use super::values::{escape_swift, resolve_streaming_adapter, swift_call_result_type, swift_client_factory_call};

/// Emit the `catch` block for an `error`-asserting test, closing the `do { … }` it follows.
///
/// ~keep When the fixture declares an expected error value, the check must match either
/// the caught error's description or its dynamic type name — never message-only — per the
/// shared contract in `declared_error_value`. With no declared value, output is byte-identical
/// to the old unconditional `// success` stub so untouched fixtures never see a diff. Which of
/// those two conventions applies, and whether Swift can ever satisfy the second, is decided once
/// by `declared_error_variant::classify` — see its doc for why Swift lands on "not yet" today
/// (swift-bridge's per-variant enum is never constructed for a real business-call failure).
fn render_error_catch_block(out: &mut String, fixture: &Fixture, errors: &[crate::core::ir::ErrorDef]) {
    use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, skip_line};
    let _ = writeln!(out, "        }} catch {{");
    match classify("swift", fixture, errors) {
        DeclaredErrorAssertion::Assert(declared) => {
            let escaped = escape_swift(declared);
            let _ = writeln!(out, "            let _errorMessage = String(describing: error)");
            let _ = writeln!(out, "            let _errorType = String(describing: type(of: error))");
            let _ = writeln!(
                out,
                "            XCTAssertTrue(_errorMessage.contains(\"{escaped}\") || _errorType.contains(\"{escaped}\"), \"expected error to mention \\\"{escaped}\\\", got message: \\(_errorMessage), type: \\(_errorType)\")"
            );
        }
        DeclaredErrorAssertion::Unsubstantiable(variant) => {
            let _ = writeln!(
                out,
                "{}",
                skip_line("            ", "//", variant, &fixture.id, "swift")
            );
        }
        DeclaredErrorAssertion::Undeclared => {
            let _ = writeln!(out, "            // success");
        }
    }
    let _ = writeln!(out, "        }}");
}

// ---------------------------------------------------------------------------
// Function-call test rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_method(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    _function_name: &str,
    _result_var: &str,
    _args: &[crate::e2e::config::ArgMapping],
    result_is_simple: bool,
    global_client_factory: Option<&str>,
    swift_first_class_map: &SwiftFirstClassMap,
    module_name: &str,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    errors: &[crate::core::ir::ErrorDef],
) {
    // Resolve per-fixture call config.
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let lang = "swift";
    let call_overrides = call_config.overrides.get(lang);

    // Merge per-call enum_fields keys into the effective enum set so that fields like "status"
    // (BatchStatus, BatchObject) are treated as enum-typed even when they are not globally
    // listed in fields_enum (they are context-dependent — BatchStatus on BatchObject but plain
    // String on ResponseObject). `with_ir_enum_map` below then rescues every enum-typed field
    // this config never mentions at all, anchored at the call's declared Rust return type. ~keep
    let mut effective_enum_fields: HashSet<String> = e2e_config.effective_fields_enum(call_config).clone();
    if let Some(o) = call_overrides {
        effective_enum_fields.extend(o.enum_fields.keys().cloned());
    }
    let call_root_type = crate::e2e::codegen::call_ir::resolve_declared_result_type(
        call_config,
        lang,
        crate::e2e::codegen::call_ir::CallIr { functions, type_defs },
    );

    // Build per-call field resolver using the effective field sets for this call.
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    let call_field_resolver = FieldResolver::new_with_swift_first_class(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        e2e_config.effective_fields_method_calls(call_config),
        &HashMap::new(),
        swift_first_class_map.clone(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone())
    .with_enum_fields(effective_enum_fields)
    .with_ir_enum_map(FieldResolver::ir_enum_fields(type_defs, enums), call_root_type.clone())
    .with_ir_collection_map(FieldResolver::ir_collection_fields(type_defs), call_root_type.clone())
    // ~keep Without this, `ir_result_field_map.root_type` stays `None`, which makes
    // `with_anchored_optional_paths` below an unconditional no-op (it early-returns on an
    // unresolved root) regardless of what paths it is given — the same gap kotlin's identical
    // wiring documents and csharp/java/typescript already avoid. An `Option<Vec<T>>` segment
    // field reached through an array-projected path (e.g. `entries[0].sections`) never matches
    // `with_ir_fields`'s bare-name-only optional set once the path crosses more than one
    // segment, so without an anchored root the per-segment accessor renderer emitted an
    // un-unwrapped `RustString`/collection access.
    .with_ir_result_fields(FieldResolver::ir_result_field_facts(type_defs, lang), call_root_type)
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    // `with_ir_fields` only proves a bare field name optional, with no path context; anchors
    // this fixture's assertion paths via the IR's real per-type walk instead, matching
    // `presentation.rs`'s existing `with_anchored_optional_paths` use. ~keep
    .with_anchored_optional_paths(fixture.assertions.iter().filter_map(|a| a.field.as_deref()));
    let field_resolver = &call_field_resolver;
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| swift_ident(&call_config.function.to_lower_camel_case()));
    // Per-call client_factory takes precedence over the global one.
    let client_factory: Option<&str> = call_overrides
        .and_then(|o| o.client_factory.as_deref())
        .or(global_client_factory);
    let result_var = call_config.effective_result_var();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs)
        .with_functions(functions);
    let target_params = recipe.target_params(lang);
    let args = recipe.args;
    // Per-call flags: base call flag OR per-language override OR global flag.
    // Also treat the call as simple when *any* language override marks it as bytes.
    // Calls like `speech()` have `result_is_bytes = true` on C/C#/Java overrides but
    // no explicit `result_is_simple` on the Swift override — yet the Swift binding
    // returns `Data` directly (not a struct), so assertions must use `result.isEmpty`
    // rather than `result.audio().toString().isEmpty`.
    let result_is_bytes_any_lang =
        call_config.result_is_bytes || call_config.overrides.values().any(|o| o.result_is_bytes);
    let result_is_simple = call_config.result_is_simple
        || call_overrides.is_some_and(|o| o.result_is_simple)
        || result_is_simple
        || result_is_bytes_any_lang;
    let result_is_array = call_config.result_is_array;
    // When the call returns `Option<T>` the Swift binding exposes the result as
    // `Optional<…>` (e.g. `getEmbeddingPreset(...) -> EmbeddingPreset?`). Bare-result
    // `is_empty`/`not_empty` assertions must use `XCTAssertNil` / `XCTAssertNotNil`
    // rather than `.toString().isEmpty`, which is undefined on opaque optionals.
    let result_is_option = call_config.result_is_option || call_overrides.is_some_and(|o| o.result_is_option);
    let result_element_is_string =
        call_config.result_element_is_string || call_overrides.is_some_and(|o| o.result_element_is_string);
    // Per-language map of array-result-field → element accessor method (e.g.
    // `structure → kind`). Empty map when no override is configured.
    let result_field_accessor: &HashMap<String, String> = call_overrides
        .map(|o| &o.result_field_accessor)
        .unwrap_or_else(|| empty_field_accessor_map());

    let method_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let is_async = call_overrides.and_then(|o| o.r#async).unwrap_or(call_config.r#async);

    // Streaming detection (call-level `streaming` opt-out is honored).
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());

    let streaming_adapter = if is_streaming && !expects_error {
        resolve_streaming_adapter(config, call_config, &function_name, client_factory)
    } else {
        None
    };
    let chunk_item_type = streaming_adapter.and_then(|adapter| adapter.item_type.as_deref());

    let collect_snippet_opt = if is_streaming && !expects_error {
        crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet_typed(
            lang,
            result_var,
            "chunks",
            chunk_item_type,
        )
    } else {
        None
    };
    // When swift has streaming-virtual-field assertions but no collect snippet
    // is available (the swift-bridge surface does not yet expose a typed
    // `chatStream` async sequence we can drain into a typed
    // a concrete stream item array), emit a skip stub rather than reference an
    // undefined `chunks` local in the assertion expressions. This keeps the
    // swift test target compiling while the binding catches up.
    if is_streaming && !expects_error && collect_snippet_opt.is_none() {
        if is_async {
            let _ = writeln!(out, "    func test{method_name}() async throws {{");
        } else {
            let _ = writeln!(out, "    func test{method_name}() throws {{");
        }
        let _ = writeln!(out, "        // {description}");
        let _ = writeln!(
            out,
            "        try XCTSkipIf(true, \"swift: streaming chunk collection is not yet supported via the swift-bridge surface (fixture: {})\")",
            fixture.id
        );
        let _ = writeln!(out, "    }}");
        return;
    }
    let collect_snippet = collect_snippet_opt.unwrap_or_default();
    // The shared streaming snippet may reference unqualified adapter item types.
    // Swift consumers import both `<Module>` (the alef-emitted first-class types)
    // AND `RustBridge` (swift-bridge generated types). Without module qualification
    // for ambiguous types, Swift fails with "'Type' is ambiguous for type lookup".
    // Qualify all bracketed type names to the first-class module type.
    let collect_snippet = if collect_snippet.is_empty() {
        collect_snippet
    } else {
        let re = Regex::new(r"\[([A-Za-z][A-Za-z0-9]*)\]").expect("valid regex");
        let module_qualifier = module_name;
        re.replace_all(&collect_snippet, |caps: &regex::Captures| {
            format!("[{}.{}]", module_qualifier, &caps[1])
        })
        .to_string()
    };

    // Detect whether this call has any json_object args that cannot be constructed
    // in Swift. Most json_object args are now handled:
    // - Scalar element types (Vec<String>, Vec<i32>, etc.) map to Swift arrays directly
    // - Array element types (Vec<DataEnum>, Vec<Struct>, etc.) are serialized to JSON strings
    // - config args are handled via options_via or default helpers
    // The only unresolvable case is a json_object arg with NO array (not a Vec) and no
    // options_via configured, which should not occur in practice. We skip in only that case.
    let has_unresolvable_json_object_arg = {
        let options_via = call_overrides.and_then(|o| o.options_via.as_deref());
        options_via.is_none()
            && args.iter().any(|a| {
                // json_object args with an element_type (Vec<T>) are always resolvable.
                // Skip only non-array json_object args without options_via.
                a.arg_type == "json_object" && a.name != "config" && a.element_type.is_none() && options_via.is_none()
            })
    };

    if has_unresolvable_json_object_arg {
        if is_async {
            let _ = writeln!(out, "    func test{method_name}() async throws {{");
        } else {
            let _ = writeln!(out, "    func test{method_name}() throws {{");
        }
        let _ = writeln!(out, "        // {description}");
        let _ = writeln!(
            out,
            "        try XCTSkipIf(true, \"swift: json_object requires options_via configuration (fixture: {})\");",
            fixture.id
        );
        let _ = writeln!(out, "    }}");
        return;
    }

    // Visitor-driven fixtures: emit a class that conforms to the generated
    // visitor protocol and wrap it via the generated visitor handle factory.
    let mut visitor_setup_lines: Vec<String> = Vec::new();
    let visitor_handle_expr: Option<String> = fixture.visitor.as_ref().map(|spec| {
        let visitor_config =
            super::super::swift_visitors::resolve_swift_visitor_config(config, call_overrides, type_defs, spec);
        super::super::swift_visitors::build_swift_visitor(
            &mut visitor_setup_lines,
            spec,
            &fixture.id,
            &visitor_config,
            module_name,
        )
    });

    // Resolve extra_args from per-call swift overrides (e.g. `nil` for optional
    // query-param arguments on list_files/list_batches that have no fixture-level
    // input field).
    let extra_args = recipe.extra_args;

    let options_via_str: Option<&str> = Some(recipe.options_via).filter(|value| *value != "kwargs");
    let options_type_str: Option<&str> = recipe.options_type;
    // Derive the Swift handle-config parsing function from the C override's
    // `c_engine_factory` field. E.g. `"CrawlConfig"` → snake → `"crawl_config_from_json"`
    // → camelCase → `"crawlConfigFromJson"`.
    let handle_config_fn_owned: Option<String> = call_config
        .overrides
        .get("c")
        .and_then(|c| c.c_engine_factory.as_deref())
        .map(|ty| format!("{}_from_json", ty.to_snake_case()).to_lower_camel_case());
    let unnamed_arg_indices: &[usize] = call_overrides.map(|o| &o.unnamed_arg_indices[..]).unwrap_or(&[]);
    let arg_name_map = call_overrides.map(|o| &o.arg_name_map);
    let streaming_request_type = resolve_streaming_adapter(config, call_config, &function_name, client_factory)
        .and_then(|adapter| adapter.request_type.as_deref())
        .map(|request_type| request_type.rsplit("::").next().unwrap_or(request_type));
    let (mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        &fixture.id,
        fixture.has_host_root_route(),
        &function_name,
        options_via_str,
        options_type_str,
        handle_config_fn_owned.as_deref(),
        visitor_handle_expr.as_deref(),
        client_factory.is_some(),
        module_name,
        unnamed_arg_indices,
        config,
        type_defs,
        fixture,
        arg_name_map,
        streaming_request_type,
        enums,
        target_params,
    );
    // Prepend visitor class declarations (before any setup lines that reference the handle).
    if !visitor_setup_lines.is_empty() {
        visitor_setup_lines.extend(setup_lines);
        setup_lines = visitor_setup_lines;
    }

    // Append extra_args to the argument list.
    let args_str = if extra_args.is_empty() {
        args_str
    } else if args_str.is_empty() {
        extra_args.join(", ")
    } else {
        format!("{args_str}, {}", extra_args.join(", "))
    };

    // When a client_factory is set, dispatch via a client instance:
    //   let client = try <FactoryType>(apiKey: "test-key", baseUrl: <mock_url>)
    //   try await client.<method>(args)
    // Otherwise fall back to free-function call (SampleCrate / non-client-factory libraries).
    let has_mock = fixture.mock_response.is_some();
    let (call_setup, call_expr) = if let Some(factory) = client_factory {
        let env_key = format!("MOCK_SERVER_{}", fixture.id.to_ascii_uppercase().replace('-', "_"));
        let mock_url = if fixture.has_host_root_route() {
            format!(
                "ProcessInfo.processInfo.environment[\"{env_key}\"] ?? (AlefE2EMockServer.baseURL + \"/fixtures/{}\")",
                fixture.id
            )
        } else {
            format!("AlefE2EMockServer.baseURL + \"/fixtures/{}\"", fixture.id)
        };
        let client_constructor = if has_mock {
            swift_client_factory_call(factory, "\"test-key\"", &mock_url)
        } else {
            // Live API: check for api_key_var; if not present use mock URL anyway.
            if let Some(env_var) = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref()) {
                format!(
                    "let _apiKey = ProcessInfo.processInfo.environment[\"{env_var}\"]\n        \
                     let _baseUrl: String? = _apiKey != nil ? nil : {mock_url}\n        \
                     {}",
                    swift_client_factory_call(factory, "_apiKey ?? \"test-key\"", "_baseUrl")
                )
            } else {
                swift_client_factory_call(factory, "\"test-key\"", &mock_url)
            }
        };
        let expr = if is_async {
            format!("try await _client.{function_name}({args_str})")
        } else {
            format!("try _client.{function_name}({args_str})")
        };
        (Some(client_constructor), expr)
    } else {
        // Free-function call (no client_factory).
        // Qualify with module name to disambiguate between high-level and swift-bridge symbols.
        let expr = if is_async {
            format!("try await {module_name}.{function_name}({args_str})")
        } else {
            format!("try {module_name}.{function_name}({args_str})")
        };
        (None, expr)
    };
    // For backwards compatibility: qualified_function_name unused when client_factory is set.
    let _ = function_name;

    if is_async {
        let _ = writeln!(out, "    func test{method_name}() async throws {{");
    } else {
        let _ = writeln!(out, "    func test{method_name}() throws {{");
    }
    let _ = writeln!(out, "        // {description}");

    if expects_error {
        // For error fixtures, setup may itself throw (e.g. config validation
        // happens at engine construction). Wrap the whole pipeline — setup
        // and the call — in a single do/catch so any throw counts as success.
        if is_async {
            // XCTAssertThrowsError is a synchronous macro; for async-throwing
            // functions use a do/catch with explicit XCTFail to enforce that
            // the throw actually happens. `await XCTAssertThrowsError(...)` is
            // not valid Swift — it evaluates `await` against a non-async expr.
            let _ = writeln!(out, "        do {{");
            for line in &setup_lines {
                let _ = writeln!(out, "            {line}");
            }
            if let Some(setup) = &call_setup {
                let _ = writeln!(out, "            {setup}");
            }
            let _ = writeln!(out, "            _ = {call_expr}");
            let _ = writeln!(out, "            XCTFail(\"expected to throw\")");
            render_error_catch_block(out, fixture, errors);
        } else {
            // Synchronous: emit setup outside (it's expected to succeed) and
            // wrap only the throwing call in XCTAssertThrowsError. If setup
            // itself throws, that propagates as the test's own failure — but
            // sync tests use `throws` so the test method itself rethrows,
            // which XCTest still records as caught. Keep this simple: use a
            // do/catch so setup-time throws also count as expected failures.
            let _ = writeln!(out, "        do {{");
            for line in &setup_lines {
                let _ = writeln!(out, "            {line}");
            }
            if let Some(setup) = &call_setup {
                let _ = writeln!(out, "            {setup}");
            }
            let _ = writeln!(out, "            _ = {call_expr}");
            let _ = writeln!(out, "            XCTFail(\"expected to throw\")");
            render_error_catch_block(out, fixture, errors);
        }
        crate::e2e::codegen::error_path_assertions::emit(out, fixture, "        // ", "swift");
        let _ = writeln!(out, "    }}");
        return;
    }

    for line in &setup_lines {
        let _ = writeln!(out, "        {line}");
    }

    // Emit client construction if a client_factory is configured.
    if let Some(setup) = &call_setup {
        let _ = writeln!(out, "        {setup}");
    }

    // Each fixture's call returns a different IR type. Override the resolver's
    // Swift first-class-map `root_type` with the call's `result_type` (looked up
    // across c/csharp/java/kotlin/go/php overrides — these are language-agnostic
    // IR type names that any backend can use to anchor field-access dispatch).
    let fixture_root_type: Option<String> = swift_call_result_type(call_config);
    let fixture_resolver = field_resolver.with_swift_root_type(fixture_root_type);
    // ~keep The anchor the EXCLUSION walk uses, kept separate from the resolver's.
    // `with_swift_root_type` assigns unconditionally, so a fixture with no explicit `result_type`
    // override leaves `fixture_resolver` with no Swift root at all -- and
    // `is_assertion_field_swift_excluded` then cannot reach a single segment, falling through to
    // the type-blind name fallback for every path. Recovering `build_swift_first_class_map`'s own
    // `result_fields` answer here rather than on the resolver is deliberate: the resolver's root
    // also decides first-class-property versus getter-call rendering, so widening it there
    // rewrites accessors for every fixture that omits the override.
    let exclusion_root_type = swift_call_result_type(call_config).or_else(|| swift_first_class_map.root_type.clone());

    // Build per-type exclusion maps from the Swift language config so that
    // assertions referencing fields or types excluded from the Swift binding
    // can be suppressed before `render_assertion` is called.
    //
    // `[languages.swift].exclude_fields` entries are in "TypeName.field_name" format.
    // `[languages.swift].exclude_types`  entries are bare IR type names.
    let swift_excluded_fields_by_type: HashMap<String, HashSet<String>> = config
        .swift
        .as_ref()
        .map(|s| {
            let mut map: HashMap<String, HashSet<String>> = HashMap::new();
            for entry in &s.exclude_fields {
                if let Some((type_name, field_name)) = entry.split_once('.') {
                    map.entry(type_name.to_string())
                        .or_default()
                        .insert(field_name.to_string());
                }
            }
            map
        })
        .unwrap_or_default();
    let swift_excluded_types: HashSet<String> = config
        .swift
        .as_ref()
        .map(|s| s.exclude_types.iter().cloned().collect())
        .unwrap_or_default();

    // Buffer assertions and collect snippet to determine if result_var is referenced.
    let mut body_buffer = String::new();

    // Add collect snippet to buffer (streaming fixtures).
    if !collect_snippet.is_empty() {
        for line in collect_snippet.lines() {
            let _ = writeln!(body_buffer, "        {line}");
        }
    }

    // A `returns_void` call binds no `result_var`, so `not_error` has nothing to assert a
    // value against the way non-void calls do. Wrap the call itself in `XCTAssertNoThrow`
    // (sync) or a do/catch that fails the test on a caught error (async, since XCTest has no
    // async-aware `XCTAssertNoThrow` overload — mirrors the do/catch this file already uses
    // for `expects_error`'s async branch, just inverted) instead of leaving `not_error`
    // vacuous. Computed up front so both the assertion loop below (which must not also emit
    // the "no result to assert on" skip comment for this specific assertion) and the
    // call-emission decision further down agree on it. ~keep
    let void_not_error = call_config.returns_void
        && fixture
            .assertions
            .iter()
            .any(|assertion| assertion.assertion_type == "not_error");

    // ~keep The non-void sibling of `void_not_error`: a call that DOES bind a `result`, but whose
    // declared assertions are `not_error` and nothing else, has no field to check `result`
    // against either. `render_not_error_assertion` (see its doc) deliberately renders only a
    // comment for a non-void call — an `XCTAssertNotNil` there would be tautological, since Swift
    // auto-promotes the declared-non-optional return type to `Optional` at the call site and the
    // assertion could never fail. That left a fixture whose ONLY assertion is `not_error` (e.g.
    // `list_validators`, `format_pptx`) with zero executable lines, refused by
    // `inert_example::inert_verdict` as `RenderedNothing` and dropped from the generated suite
    // entirely, even though the call genuinely throwing IS the check the fixture asked for. Fires
    // only when EVERY declared assertion is `not_error` — a fixture that pairs `not_error` with a
    // real field assertion keeps binding `result` unchanged, since that assertion still needs it.
    let non_void_not_error_only = !call_config.returns_void
        && !fixture.assertions.is_empty()
        && fixture
            .assertions
            .iter()
            .all(|assertion| assertion.assertion_type == "not_error");

    // Add assertions to buffer.
    let mut void_skip_comment_emitted = false;
    for assertion in &fixture.assertions {
        // Skip assertions for Void-returning functions (they don't produce a result to assert on).
        // Only emit this comment once (not per assertion).
        if call_config.returns_void {
            if assertion.assertion_type == "not_error" {
                // Handled after this loop: the call itself gets wrapped in a real assertion
                // instead (see `void_not_error` above), so nothing is skipped here.
                continue;
            }
            if !void_skip_comment_emitted {
                let _ = writeln!(
                    body_buffer,
                    "        // skipped: void-returning function has no result to assert on"
                );
                void_skip_comment_emitted = true;
            }
            continue;
        }

        // A `not_error`-only non-void fixture: skip the vacuous comment here too, the same way
        // the void branch above does — the call gets wrapped in a real assertion after this loop
        // (see `non_void_not_error_only`) instead.
        if non_void_not_error_only && assertion.assertion_type == "not_error" {
            continue;
        }

        // Skip assertions whose field path traverses a field or resolves to a
        // type that is excluded from the Swift binding.  This prevents compile
        // errors like "value of type 'ExtractedDocumentRef' has no member
        // 'extractedKeywords'" when a fixture assertion exercises a feature
        // (e.g. keyword extraction) whose types are excluded from the Swift
        // binding via `[languages.swift].exclude_types` /
        // `[languages.swift].exclude_fields` in `alef.toml`.
        if let Some(f) = assertion.field.as_deref()
            && !f.is_empty()
        {
            let resolved_f = fixture_resolver.resolve(f);
            if is_assertion_field_swift_excluded(
                resolved_f,
                exclusion_root_type.as_deref(),
                &swift_first_class_map.field_types,
                &swift_excluded_fields_by_type,
                &swift_excluded_types,
            ) {
                let _ = writeln!(
                    body_buffer,
                    "        // skipped: {}",
                    FieldSkip::ExcludedFromSwiftBinding.message(f)
                );
                continue;
            }
        }
        let mut assertion_out = String::new();
        render_assertion(
            &mut assertion_out,
            assertion,
            result_var,
            &fixture_resolver,
            result_is_simple,
            result_is_array,
            result_is_option,
            result_element_is_string,
            result_field_accessor,
            is_streaming,
            call_config.returns_void,
        );
        // Module-qualify swift-bridge-ambiguous DTO type names that appear in
        // streaming-virtual assertion expressions (e.g. `[StreamToolCall]`,
        // `[ToolCall]`). Both `<Module>` (first-class Codable struct) and
        // `RustBridge` (swift-bridge opaque class) export the same identifier,
        // so unqualified usage fails Swift compilation with "X is ambiguous for
        // type lookup". Mirrors the stream item type qualification in
        // `render_test_method`.
        for unqualified in ["StreamToolCall", "ToolCall"] {
            assertion_out =
                assertion_out.replace(&format!("[{unqualified}]"), &format!("[{module_name}.{unqualified}]"));
        }
        body_buffer.push_str(&assertion_out);
    }
    crate::e2e::codegen::fail_on_unavailable_field_markers(&body_buffer, "swift", &fixture.id, &fixture.assertions);
    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&body_buffer, "swift", &fixture.id);
    // A `not_error`-only void OR non-void fixture gets its assertion here, before `inert_verdict`
    // below — rather than as the bare call emitted at the call-emission site further down — so
    // that a real executable line is already in `body_buffer` by the time `inert_verdict` looks
    // for one. Before this, `body_buffer` held only the "no result to assert on" skip comment for
    // every void fixture, `inert_verdict` saw no executable line, and substituted an
    // unconditional `try XCTSkipIf(true, ...)` in its place — the fixture's test method never
    // ran the check it declared, it silently skipped itself instead. ~keep
    // `non_void_not_error_only` reuses the exact same templates: `XCTAssertNoThrow`/do-catch wraps
    // any throwing expression and discards its result whether or not that expression has a return
    // value, so the void machinery works unmodified for the non-void case too. ~keep
    if void_not_error || non_void_not_error_only {
        let template = if is_async {
            "swift/void_not_error_async.jinja"
        } else {
            "swift/void_not_error_sync.jinja"
        };
        body_buffer.push_str(&crate::e2e::template_env::render(
            template,
            minijinja::context! { call_expr => call_expr },
        ));
    }
    // ~keep Order relative to the call-emission decision below no longer matters -- that
    // decision now reads `fixture.assertions` directly rather than the rendered `body_buffer` --
    // but this still has to run after every assertion has been appended, since it is exactly the
    // rendered text (including any `skipped:` markers) it inspects. Swift has no formatter that
    // objects to a body which runs a real stream and then ends on a lone `skipped:` comment, so
    // this shape shipped green and would have kept shipping green.
    let refusal = inert_example::inert_verdict(&body_buffer, "swift", &fixture.id, &fixture.assertions);

    // Decide how to emit the call based on return type and whether the fixture declares any
    // assertion at all.
    // - void returns with a `not_error` assertion: the call already went into `body_buffer`
    //   above, wrapped in a real assertion — emit nothing more here.
    // - non-void with ONLY a `not_error` assertion: same — already wrapped and emitted above.
    // - void returns otherwise: emit bare call
    // - non-void with at least one declared assertion: bind with `let result = `
    // - non-void with NO declared assertions at all: discard with `_ = ` (the "just call it"
    //   smoke-test contract `inert_example`'s own doc names)
    //
    // ~keep Was `body_buffer.contains(result_var)` — whether the RENDERED text happened to
    // mention `result` — which made the binding choice hostage to what each assertion arm chose
    // to emit. `not_error`'s own fix (see `not_error_assertion.rs`) collapsed its output to a
    // comment that never mentions `result`, and for a `not_error`-only fixture that flipped this
    // check to `false` and downgraded `let result = try await SampleExtract.extract(...)` to
    // `_ = try await SampleExtract.extract(...)` — same call, same fixture input JSON, but a
    // documentation-facing example snippet that no longer reads as "call it and get `result`"
    // (`swift_unified_extract_single_fixture_emits_input_json` caught this). The binding a
    // fixture deserves is a property of what it DECLARED, not an accident of how one particular
    // assertion type chose to render this month — `fixture.assertions.is_empty()` asks the first
    // question directly and cannot be perturbed by a later change to any single assertion arm's
    // wording.
    if void_not_error || non_void_not_error_only {
        // Already emitted into `body_buffer` above.
    } else if call_config.returns_void {
        let _ = writeln!(out, "        {call_expr}");
    } else if fixture.assertions.is_empty() {
        let _ = writeln!(out, "        _ = {call_expr}");
    } else {
        let _ = writeln!(out, "        let {result_var} = {call_expr}");
    }

    // ~keep The call above is kept even when the example is refused: it is the one thing here that
    // can still fail on its own (every generated test method is `throws`, so a throwing call fails
    // the test), and deleting it would remove coverage that runs today. What is replaced is the
    // part that made the method report a PASS while checking nothing.
    if let Some(refusal) = &refusal {
        inert_example::record_refusal(refusal);
        body_buffer = render_swift_refusal(&body_buffer, refusal);
    }

    // Emit the buffered body (assertions and collect snippet).
    out.push_str(&body_buffer);

    // Emit teardown for test backends: unregister to prevent leaking into subsequent tests.
    for arg in args {
        if arg.arg_type == "test_backend"
            && let Some(trait_name) = &arg.trait_name
            && let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name)
        {
            let unregister_fn = format!("unregister{}", trait_bridge.trait_name.to_upper_camel_case());
            // Use the actual plugin name from fixture.input["name"] or default to fixture.id,
            // matching what the stub's `name` property declares. This ensures unregister()
            // matches the registered backend name.
            let plugin_name = fixture
                .input
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&fixture.id);
            let _ = writeln!(out, "        try? {module_name}.{unregister_fn}(\"{plugin_name}\")");
        }
    }

    let _ = writeln!(out, "    }}");
}

/// The body emitted in place of one whose declared assertions all funnelled into skip markers.
///
/// ~keep Which refusal is emitted follows who can fix it, exactly as in `ruby/examples.rs`. An
/// unresolved field path is the consumer's to repair, so it gets `XCTFail` — under the default
/// strict setting the run has already failed, and the deliberately disarmed run must still not go
/// green. Everything else is alef's generator debt or a swift-bridge limit that no consumer edit
/// clears; failing their suite for it would only force a blanket opt-out, so it gets `XCTSkipIf`,
/// which XCTest reports as skipped and never as a pass. Both spellings are already emitted
/// elsewhere in this file, so neither introduces a construct the generated project cannot build.
fn render_swift_refusal(markers: &str, refusal: &InertExample) -> String {
    let reason = escape_swift(&refusal.reason());
    let statement = match refusal.cause {
        InertCause::UnresolvedFieldPath => format!("        XCTFail(\"{reason}\")\n"),
        InertCause::AwaitedOrLimited | InertCause::RenderedNothing => {
            format!("        try XCTSkipIf(true, \"{reason}\")\n")
        }
    };
    inert_example::refusal_body(markers, &statement)
}

mod field_exclusion;
use field_exclusion::is_assertion_field_swift_excluded;

#[cfg(test)]
mod error_catch_block_tests {
    use super::render_error_catch_block;
    use crate::core::ir::{ErrorDef, ErrorVariant};
    use crate::e2e::fixture::{Assertion, Fixture};

    fn fixture_with_declared_error(value: &str) -> Fixture {
        Fixture {
            id: "declares_error".to_string(),
            assertions: vec![Assertion {
                assertion_type: "error".to_string(),
                value: Some(serde_json::Value::String(value.to_string())),
                ..Assertion::default()
            }],
            ..Fixture::default()
        }
    }

    fn coded_error_def(variant_name: &str) -> ErrorDef {
        ErrorDef {
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: variant_name.to_string(),
                error_code: Some(100),
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    #[test]
    fn no_declared_value_renders_the_success_stub() {
        let fixture = Fixture {
            id: "no_error".to_string(),
            ..Fixture::default()
        };
        let mut out = String::new();
        render_error_catch_block(&mut out, &fixture, &[]);
        assert_eq!(out, "        } catch {\n            // success\n        }\n");
    }

    /// With no `errors` IR supplied, a value cannot be recognised as a known variant name, so it
    /// renders exactly like a message-style value always did before this fix.
    #[test]
    fn message_style_value_renders_the_message_or_type_assertion() {
        let fixture = fixture_with_declared_error("BadRequest");
        let mut out = String::new();
        render_error_catch_block(&mut out, &fixture, &[]);
        assert!(
            out.contains("_errorMessage.contains(\"BadRequest\") || _errorType.contains(\"BadRequest\")"),
            "got: {out}"
        );
    }

    /// The defect this fix closes: a declared value that names a real `ErrorVariant` —
    /// swift-bridge's per-variant enum is never constructed for a real business-call failure —
    /// must render the registered skip, not an `XCTAssertTrue` that can never pass.
    #[test]
    fn declared_value_naming_a_known_variant_renders_the_registered_skip() {
        let fixture = fixture_with_declared_error("Authentication");
        let errors = vec![coded_error_def("Authentication")];
        let mut out = String::new();
        render_error_catch_block(&mut out, &fixture, &errors);
        assert_eq!(
            out,
            "        } catch {\n            // skipped: declared error variant 'Authentication' not yet preserved \
             as a distinct identity by this backend's generator\n        }\n"
        );
        assert!(
            !out.contains("XCTAssertTrue"),
            "must not render an assertion that can never pass, got: {out}"
        );
    }
}

#[cfg(test)]
mod inert_example_refusal_tests {
    use super::render_test_method;
    use crate::e2e::codegen::inert_example::take_inert_examples;
    use crate::e2e::config::{CallConfig, E2eConfig};
    use crate::e2e::field_access::SwiftFirstClassMap;
    use crate::e2e::fixture::{Assertion, Fixture};

    fn assertion(assertion_type: &str, field: &str) -> Assertion {
        Assertion {
            assertion_type: assertion_type.to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::json!("x")),
            ..Default::default()
        }
    }

    /// A non-empty `result_fields` set is what arms the availability oracle: with it empty the
    /// resolver is deliberately permissive and no field is ever rejected. ~keep
    fn field_gated_e2e_config() -> E2eConfig {
        E2eConfig {
            result_fields: std::collections::HashSet::from(["content".to_string()]),
            call: CallConfig {
                function: "process".to_string(),
                result_var: "result".to_string(),
                returns_result: true,
                ..CallConfig::default()
            },
            ..E2eConfig::default()
        }
    }

    fn render(assertions: Vec<Assertion>, fixture_id: &str) -> String {
        let fixture = Fixture {
            id: fixture_id.to_string(),
            description: "swift refusal fixture".to_string(),
            assertions,
            ..Fixture::default()
        };
        let mut out = String::new();
        render_test_method(
            &mut out,
            &fixture,
            &field_gated_e2e_config(),
            "process",
            "result",
            &[],
            false,
            None,
            &SwiftFirstClassMap::default(),
            "SampleModule",
            &crate::core::config::ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        );
        out
    }

    /// CONTROL, asserted first: a field the oracle resolves still renders its real check, and no
    /// refusal is recorded. An over-broad refusal here would silently delete coverage that runs
    /// today — the same defect pointing the other way. ~keep
    #[test]
    fn a_resolvable_assertion_is_published_unchanged() {
        let _ = take_inert_examples();

        let out = render(vec![assertion("equals", "content")], "swift_control");

        assert!(
            out.contains("XCTAssert"),
            "the renderable assertion must still be emitted, got:\n{out}"
        );
        assert!(
            !out.contains("XCTSkipIf(true, \"alef "),
            "a live example must not be refused, got:\n{out}"
        );
        assert!(
            take_inert_examples().is_empty(),
            "nothing may be recorded for a live example"
        );
    }

    /// The blocker: every declared assertion funnels into a skip marker, so the method called the
    /// binding and then checked nothing. Swift has no formatter that objects, so this shipped as a
    /// permanent green. It must now report as skipped, and carry the markers with it.
    #[test]
    fn an_example_whose_every_assertion_skips_is_refused_as_a_skipped_test() {
        let _ = take_inert_examples();

        let out = render(
            vec![
                assertion("equals", "nonexistent_field"),
                assertion("equals", "another_missing_field"),
            ],
            "swift_all_skipped",
        );

        assert!(
            out.contains("XCTFail(\"alef resolved no assertion for fixture `swift_all_skipped`"),
            "an unresolved field path must be refused with a FAILING check, got:\n{out}"
        );
        assert!(
            out.contains("skipped:") && out.contains("nonexistent_field") && out.contains("another_missing_field"),
            "the markers must be carried into the refusal, got:\n{out}"
        );
        assert!(
            out.contains("SampleModule.process("),
            "the call itself still fails on throw and must not be deleted, got:\n{out}"
        );
        let refusals = take_inert_examples();
        assert_eq!(refusals.len(), 1, "the refusal must be recorded once for the summary");
        assert_eq!(refusals[0].fixture_id, "swift_all_skipped");
    }

    /// alef's own generator debt is not the consumer's to fix, so it gets `XCTSkipIf` rather than
    /// `XCTFail`: failing a consumer's build for a gap no fixture edit clears is what forces the
    /// blanket opt-out this whole mechanism exists to avoid.
    #[test]
    fn generator_debt_is_refused_as_a_skip_rather_than_a_failure() {
        let _ = take_inert_examples();

        let out = render(
            vec![Assertion {
                assertion_type: "equals".to_string(),
                field: Some("nonexistent_field".to_string()),
                value: Some(serde_json::json!("x")),
                skip: Some(crate::e2e::fixture::AssertionSkip::All(true)),
                ..Default::default()
            }],
            "swift_generator_debt",
        );

        assert!(
            out.contains("XCTSkipIf(true, \"alef rendered no runnable expectation for fixture `swift_generator_debt`"),
            "acknowledged debt must be parked as skipped, got:\n{out}"
        );
        assert!(
            !out.contains("XCTFail(\"alef "),
            "alef's own debt must not fail a consumer's suite, got:\n{out}"
        );
        assert_eq!(take_inert_examples().len(), 1);
    }

    /// CONTROL: a fixture that declares NO assertions is the deliberate "just call it" smoke
    /// contract and must be published exactly as before. ~keep
    #[test]
    fn a_fixture_with_no_declared_assertions_keeps_its_smoke_test_shape() {
        let _ = take_inert_examples();

        let out = render(Vec::new(), "swift_smoke_only");

        assert!(
            out.contains("SampleModule.process("),
            "the call must still be emitted, got:\n{out}"
        );
        assert!(
            !out.contains("XCTSkipIf(true, \"alef ") && !out.contains("XCTFail(\"alef "),
            "a fixture with no assertions must never be refused, got:\n{out}"
        );
        assert!(take_inert_examples().is_empty());
    }
}

#[cfg(test)]
#[path = "test_method/ir_result_fields_wiring_tests.rs"]
mod ir_result_fields_wiring_tests;
