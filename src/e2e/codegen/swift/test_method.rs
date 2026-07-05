use crate::codegen::keywords::swift_ident;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::{FieldResolver, SwiftFirstClassMap};
use crate::e2e::fixture::Fixture;
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use regex::Regex;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use super::args::build_args_and_setup;
use super::assertions::render_assertion;
use super::empty_field_accessor_map;
use super::values::{resolve_streaming_adapter, swift_call_result_type, swift_client_factory_call};

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
) {
    // Resolve per-fixture call config.
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Build per-call field resolver using the effective field sets for this call.
    let call_field_resolver = FieldResolver::new_with_swift_first_class(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        e2e_config.effective_fields_method_calls(call_config),
        &HashMap::new(),
        swift_first_class_map.clone(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone());
    let field_resolver = &call_field_resolver;
    let enum_fields = e2e_config.effective_fields_enum(call_config);
    let lang = "swift";
    let call_overrides = call_config.overrides.get(lang);
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| swift_ident(&call_config.function.to_lower_camel_case()));
    // Per-call client_factory takes precedence over the global one.
    let client_factory: Option<&str> = call_overrides
        .and_then(|o| o.client_factory.as_deref())
        .or(global_client_factory);
    let result_var = &call_config.result_var;
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs);
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

    // Merge per-call enum_fields keys into the effective enum set so that
    // fields like "status" (BatchStatus, BatchObject) are treated as enum-typed
    // even when they are not globally listed in fields_enum (they are context-
    // dependent — BatchStatus on BatchObject but plain String on ResponseObject).
    let effective_enum_fields: Cow<HashSet<String>> = {
        let per_call = call_overrides.map(|o| &o.enum_fields);
        if let Some(pc) = per_call {
            if !pc.is_empty() {
                let mut merged = enum_fields.clone();
                merged.extend(pc.keys().cloned());
                Cow::Owned(merged)
            } else {
                Cow::Borrowed(enum_fields)
            }
        } else {
            Cow::Borrowed(enum_fields)
        }
    };

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
            let _ = writeln!(out, "        }} catch {{");
            let _ = writeln!(out, "            // success");
            let _ = writeln!(out, "        }}");
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
            let _ = writeln!(out, "        }} catch {{");
            let _ = writeln!(out, "            // success");
            let _ = writeln!(out, "        }}");
        }
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

    // Add assertions to buffer.
    let mut void_skip_comment_emitted = false;
    for assertion in &fixture.assertions {
        // Skip assertions for Void-returning functions (they don't produce a result to assert on).
        // Only emit this comment once (not per assertion).
        if call_config.returns_void {
            if !void_skip_comment_emitted {
                let _ = writeln!(
                    body_buffer,
                    "        // skipped: void-returning function has no result to assert on"
                );
                void_skip_comment_emitted = true;
            }
            continue;
        }

        // Skip assertions whose field path traverses a field or resolves to a
        // type that is excluded from the Swift binding.  This prevents compile
        // errors like "value of type 'ExtractedDocumentRef' has no member
        // 'extractedKeywords'" when a fixture assertion exercises a feature
        // (e.g. keyword extraction) whose types are excluded from the Swift
        // binding via `[languages.swift].exclude_types` /
        // `[languages.swift].exclude_fields` in `alef.toml`.
        if let Some(f) = assertion.field.as_deref() {
            if !f.is_empty() {
                let resolved_f = fixture_resolver.resolve(f);
                if is_assertion_field_swift_excluded(
                    resolved_f,
                    fixture_resolver.swift_root_type().map(String::as_str),
                    &swift_first_class_map.field_types,
                    &swift_excluded_fields_by_type,
                    &swift_excluded_types,
                ) {
                    let _ = writeln!(
                        body_buffer,
                        "        // skipped: field '{f}' references a field or type excluded from the Swift binding"
                    );
                    continue;
                }
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
            &effective_enum_fields,
            is_streaming,
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

    // Decide how to emit the call based on return type and whether result is referenced.
    // - void returns: emit bare call
    // - non-void with result referenced: bind with `let result = `
    // - non-void without result referenced: discard with `_ = `
    if call_config.returns_void {
        let _ = writeln!(out, "        {call_expr}");
    } else if body_buffer.contains(result_var) {
        let _ = writeln!(out, "        let {result_var} = {call_expr}");
    } else {
        let _ = writeln!(out, "        _ = {call_expr}");
    }

    // Emit the buffered body (assertions and collect snippet).
    out.push_str(&body_buffer);

    // Emit teardown for test backends: unregister to prevent leaking into subsequent tests.
    for arg in args {
        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
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
        }
    }

    let _ = writeln!(out, "    }}");
}

/// Returns `true` when the assertion `field_path` traverses a field or resolves to a
/// type that is excluded from the Swift binding.
///
/// Walks the dot/bracket-separated path segments through `field_types` (a map from owner
/// type → field name → inner named type, populated in `build_swift_first_class_map`),
/// starting from `root_type`.  At each segment it checks:
///
/// 1. Whether `(current_type, segment)` appears in `excluded_fields_by_type` (built
///    from `[languages.swift].exclude_fields` entries of the form `"TypeName.field_name"`).
/// 2. Whether the named type the segment advances into appears in `excluded_types` (built
///    from `[languages.swift].exclude_types`).
///
/// As a fallback (used when `root_type` cannot be resolved — `swift_call_result_type`
/// only returns a type when the call has an explicit `result_type` override), any path
/// segment whose name matches the field part of *any* `exclude_fields` entry is also
/// treated as excluded. The excluded leaf names in this binding (`extracted_keywords`,
/// `internal_document`, `ocr_internal_document`, …) are unique to `ExtractedDocument`,
/// so this name-based check does not over-skip in practice.
///
/// Returns `false` only when both exclusion maps are empty.
fn is_assertion_field_swift_excluded(
    field_path: &str,
    root_type: Option<&str>,
    field_types: &HashMap<String, HashMap<String, String>>,
    excluded_fields_by_type: &HashMap<String, HashSet<String>>,
    excluded_types: &HashSet<String>,
) -> bool {
    if excluded_fields_by_type.is_empty() && excluded_types.is_empty() {
        return false;
    }
    // Split the field path on '.', '[', ']', discarding empty tokens and tokens
    // that are pure numeric indices (e.g. "0" from "results[0].extracted_keywords").
    let segments: Vec<&str> = field_path
        .split(['.', '[', ']'])
        .filter(|s| !s.is_empty() && !s.chars().all(|c: char| c.is_ascii_digit()))
        .collect();
    // Name-based fallback: any segment matching an excluded field leaf name.
    for segment in &segments {
        if excluded_fields_by_type.values().any(|fields| fields.contains(*segment)) {
            return true;
        }
    }
    // Type-aware walk (precise when `root_type` is known).
    let mut current_type: Option<String> = root_type.map(|s| s.to_string());
    for segment in segments {
        let Some(owner_str) = current_type.as_deref() else {
            break;
        };
        // 1. Explicitly excluded (owner_type, field_name) pair.
        if excluded_fields_by_type
            .get(owner_str)
            .is_some_and(|fields| fields.contains(segment))
        {
            return true;
        }
        // Advance the type cursor to the named type that `segment` leads into.
        let next: Option<String> = field_types.get(owner_str).and_then(|m| m.get(segment).cloned());
        // 2. The resolved target type is excluded from the Swift binding.
        if let Some(ref next_type) = next {
            if excluded_types.contains(next_type.as_str()) {
                return true;
            }
        }
        current_type = next;
    }
    false
}
