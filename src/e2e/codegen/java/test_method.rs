//! Java test-method rendering.
//!
//! ~keep This file is already over the repo's 1,000-line file-modularization cap. The
//! `not_error_may_assert_presence` unification (routing `not_error` through
//! `not_error_presence::may_assert_presence`) added one call-site argument to
//! `render_assertion` plus the shared computation feeding it — a small, bounded amount of
//! production wiring, not new unrelated functionality.

use crate::core::config::ResolvedCrateConfig;
use crate::core::config::extras::AdapterConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::escape_java;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use heck::{ToLowerCamelCase, ToUpperCamelCase};

use super::args::{JavaArgsContext, build_args_and_setup};
use super::assertions::{fractional_scalar_fields, render_assertion};
use super::http::render_http_test_method;
use super::values::{java_builder_expression, json_to_java};
use super::visitor::{apply_java_visitor_arg, build_java_visitor, java_visitor_binding};
use crate::e2e::codegen::inert_example::{self, InertCause};

/// The body emitted in place of one whose declared assertions all funnelled into skip markers.
///
/// ~keep Which refusal is emitted follows who can fix it, exactly as in `ruby/examples.rs`. An
/// unresolved field path is the consumer's to repair, so it gets an `Assertions.fail` naming the
/// fixture — under the default strict setting the run has already failed, and the deliberately
/// disarmed run must still not go green. Everything else is alef's generator debt or a JNI/ABI
/// limit no consumer edit clears; failing their suite for it would only force a blanket opt-out,
/// so it gets JUnit's `Assumptions.assumeTrue(false, ..)`, which reports the test as skipped and
/// never as a pass. Both are fully qualified, which is what `test_method.jinja`'s fixed import set
/// makes necessary — the same spelling the api-key assumption above already uses.
fn render_java_refusal(markers: &str, refusal: &inert_example::InertExample) -> String {
    let reason = escape_java(&refusal.reason());
    let statement = match refusal.cause {
        InertCause::UnresolvedFieldPath => format!("        org.junit.jupiter.api.Assertions.fail(\"{reason}\");\n"),
        InertCause::AwaitedOrLimited | InertCause::RenderedNothing => {
            format!("        org.junit.jupiter.api.Assumptions.assumeTrue(false, \"{reason}\");\n")
        }
    };
    inert_example::refusal_body(markers, &statement)
}

/// Render the JUnit assertion that checks a declared `error` fixture value against either
/// the thrown exception's message or its simple class name — or, when the declared value
/// names a real error variant this backend's binding cannot substantiate, the registered
/// skip instead of an assertion that can never pass.
///
/// ~keep Mirrors the Rust/Python/Go backends' disjunction (see
/// `crate::e2e::codegen::declared_error_value`): fixture authors name either a message
/// substring (config-validation fixtures) or a type-name prefix (API-error fixtures) in
/// the assertion's value, never both conventions at once. Checking `getMessage()` OR
/// the exception's simple class name lets this single code path serve both. Which of those
/// two conventions applies, and whether Java can ever satisfy the second, is decided once by
/// `declared_error_variant::classify`.
fn declared_error_value_check(fixture: &Fixture, errors: &[crate::core::ir::ErrorDef]) -> Option<String> {
    use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, skip_line};
    match classify("java", fixture, errors) {
        DeclaredErrorAssertion::Undeclared => None,
        DeclaredErrorAssertion::Assert(declared) => {
            let escaped = escape_java(declared);
            Some(format!(
                "        assertTrue(thrown.getMessage() != null && thrown.getMessage().contains(\"{escaped}\") \
|| thrown.getClass().getSimpleName().contains(\"{escaped}\"), \"expected error to match: {escaped}\");"
            ))
        }
        DeclaredErrorAssertion::Unsubstantiable(variant) => {
            Some(skip_line("        ", "//", variant, &fixture.id, "java"))
        }
    }
}

fn ensure_assertion_line_ending(assertions: &mut String) {
    if !assertions.is_empty() && !assertions.ends_with('\n') {
        assertions.push('\n');
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_method(
    out: &mut String,
    fixture: &Fixture,
    class_name: &str,
    _function_name: &str,
    _result_var: &str,
    _args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    nested_types: &std::collections::HashMap<String, String>,
    nested_types_optional: bool,
    adapters: &[AdapterConfig],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    errors: &[crate::core::ir::ErrorDef],
) {
    // Delegate HTTP fixtures to the HTTP-specific renderer.
    if let Some(http) = &fixture.http {
        render_http_test_method(out, fixture, http);
        return;
    }

    // Resolve per-fixture call config (supports named calls via fixture.call field).
    // Use resolve_call_for_fixture to support auto-routing via select_when.
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Per-call field resolver: overrides the category-level resolver when this call
    // declares its own result_fields / fields / fields_optional / fields_array.
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    let lang = "java";
    // Anchor the IR-derived enum classification (`with_ir_enum_map`) at the call's declared
    // Rust return type, mirroring the csharp/kotlin/swift/dart/gleam e2e generators. Without
    // this, `field_is_enum` in `assertions.rs` can only see the hand-maintained `fields_enum`
    // config, so a real Java enum field nobody listed there (e.g. a recursive struct's own
    // enum field, reached only through the parent's field path) silently falls back to a plain
    // `assertEquals(String, EnumType)` that can never pass. ~keep
    let call_root_type = crate::e2e::codegen::call_ir::resolve_declared_result_type(
        call_config,
        lang,
        crate::e2e::codegen::call_ir::CallIr { functions, type_defs },
    );
    // Enum types the Java binding backend renders as a tagged/untagged-union wrapper class
    // rather than a plain `enum` with `getValue()` — the exact predicate `gen_enum_class` uses
    // to pick its branch, reused here so `field_is_enum` in `assertions.rs` can never disagree
    // with what the binding backend actually emitted (see `emits_get_value`'s doc). ~keep
    let java_wrapper_enum_names: std::collections::HashSet<String> = enums
        .iter()
        .filter(|enum_def| !crate::backends::java::gen_bindings::emits_get_value(enum_def))
        .map(|enum_def| enum_def.name.clone())
        .collect();
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone())
    .with_enum_fields(e2e_config.effective_fields_enum(call_config).clone())
    .with_ir_enum_map(FieldResolver::ir_enum_fields(type_defs, enums), call_root_type.clone())
    .with_java_wrapper_enum_names(java_wrapper_enum_names)
    // Mirrors the csharp/kotlin/swift/rust e2e generators (see `FieldResolver::
    // ir_collection_fields`'s doc): without this, `is_collection_root` can only see the
    // hand-maintained `fields_array`/`fields_optional` config, so a collection field nothing
    // ever indexes into (no per-element fixture path) has no IR-backed signal at all.
    .with_ir_collection_map(FieldResolver::ir_collection_fields(type_defs), call_root_type.clone())
    .with_ir_result_fields(FieldResolver::ir_result_field_facts(type_defs, lang), call_root_type)
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    // `with_ir_fields` only proves a bare field name optional, with no path context; anchors
    // this fixture's assertion paths via the IR's real per-type walk instead, matching
    // `presentation.rs`'s existing `with_anchored_optional_paths` use. ~keep
    .with_anchored_optional_paths(fixture.assertions.iter().filter_map(|a| a.field.as_deref()));
    let field_resolver = &call_field_resolver;
    let effective_enum_fields = e2e_config.effective_fields_enum(call_config);
    let enum_fields = effective_enum_fields;
    let call_overrides = call_config.overrides.get(lang);
    let effective_function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.to_lower_camel_case());
    let function_name = effective_function_name.as_str();
    let result_var = call_config.effective_result_var();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs)
        .with_functions(functions);
    let target_params = recipe.target_params(lang);
    let args: &[crate::e2e::config::ArgMapping] = recipe.args;

    let method_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Resolve per-fixture options_type: prefer the java call override, fall back to
    // class-level, then to any other language's options_type for the same call (the
    // generated Java POJO class name matches the Rust type name across bindings, so
    // mirroring the C/csharp/go option lets us auto-emit `Type.fromJson(json)` without
    // requiring an explicit Java override per call).
    let effective_options_type: Option<String> = recipe
        .options_type
        .map(str::to_string)
        .or_else(|| options_type.map(str::to_string))
        .or_else(|| {
            recipe
                .compatible_options_type(&["csharp", "c", "go", "php", "python"])
                .map(str::to_string)
        });
    let effective_options_type = effective_options_type.as_deref();
    // When options_type is resolvable but no explicit options_via is given for Java,
    // default to "from_json" so the typed-request arg is emitted as
    // `Type.fromJson(json)` rather than the raw JSON string. The Java backend exposes
    // a static `fromJson(String)` factory on every record type (Stage A).
    let auto_from_json = effective_options_type.is_some()
        && call_overrides.and_then(|o| o.options_via.as_deref()).is_none()
        && e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.options_via.as_deref())
            .is_none();

    // Resolve client_factory: prefer call-level java override, fall back to file-level java override.
    let client_factory: Option<String> = call_overrides.and_then(|o| o.client_factory.clone()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.client_factory.clone())
    });

    // Resolve options_via: "kwargs" (default), "from_json", "json", "dict".
    // Auto-default to "from_json" when an options_type is resolvable and no explicit
    // options_via is configured — this lets typed-request args emit `Type.fromJson(json)`
    // even when alef.toml only declares the type in another binding's override block.
    let options_via: String = call_overrides
        .and_then(|o| o.options_via.clone())
        .or_else(|| e2e_config.call.overrides.get(lang).and_then(|o| o.options_via.clone()))
        .unwrap_or_else(|| {
            if auto_from_json {
                "from_json".to_string()
            } else {
                "kwargs".to_string()
            }
        });

    // Resolve per-fixture result_is_simple and result_is_bytes from the call override.
    let effective_result_is_simple =
        call_overrides.is_some_and(|o| o.result_is_simple) || call_config.result_is_simple || result_is_simple;
    let effective_result_is_bytes = call_overrides.is_some_and(|o| o.result_is_bytes);
    // Resolve result_is_option: when the Rust function returns `Option<T>`, the Java
    // facade typically returns `@Nullable T` (via `.orElse(null)`).  Bare-result
    // is_empty/not_empty assertions must use `assertNull/assertNotNull` rather than
    // calling `.isEmpty()` on the nullable reference, which is undefined for record
    // types (mirrors the Kotlin / Zig codegen behaviour).
    let effective_result_is_option = call_overrides.is_some_and(|o| o.result_is_option) || call_config.result_is_option;

    // Check if this test needs ObjectMapper deserialization for json_object args.
    let needs_deser = args.iter().any(|arg| {
        if arg.arg_type != "json_object" {
            return false;
        }
        let val = super::super::resolve_field(&fixture.input, &arg.field);
        !val.is_null()
            && !val.is_array()
            && crate::e2e::codegen::recipe::json_object_constructor_type(arg, effective_options_type, val).is_some()
    });

    // IR-derived enum classification for `java_builder_expression`'s json_object args, keyed
    // by owner type so a field name that means different things on different structs is never
    // conflated. Computed once here and reused across every arg's builder expression below. ~keep
    let ir_enum_map = FieldResolver::ir_enum_fields(type_defs, enums);

    // Emit builder expressions for json_object args.
    let mut builder_expressions = String::new();
    if needs_deser {
        for arg in args {
            if arg.arg_type == "json_object" {
                let val = super::super::resolve_field(&fixture.input, &arg.field);
                if !val.is_null() && !val.is_array() {
                    let Some(opts_type) =
                        crate::e2e::codegen::recipe::json_object_constructor_type(arg, effective_options_type, val)
                    else {
                        continue;
                    };
                    if options_via == "from_json" {
                        // Build the typed POJO via `JsonUtil.fromJson(json, Type.class)`.
                        // The Java backend centralizes JSON deserialization in JsonUtil rather
                        // than per-DTO static methods.  Java uses snake_case wire format
                        // (matches Rust's serde default), so pass through fixture keys as-is.
                        let normalized = super::super::transform_json_keys_for_language(val, "snake_case");
                        let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                        // A full literal expression -- quoted, or `+`-chunked when `json_str`
                        // is long enough to threaten the JVM's 65535-byte constant cap -- not
                        // just escaped content. See `values::java_string_literal`.
                        let literal = super::values::java_string_literal(&json_str);
                        let var_name = &arg.name;
                        if crate::e2e::codegen::value_contains_mock_url_placeholder(&normalized) {
                            let env_key = crate::e2e::codegen::mock_url_env_key(&fixture.id);
                            builder_expressions.push_str(&format!(
                                "        String {var_name}MockBaseUrl = System.getProperty(\"mockServer.{fixture_id}\", System.getenv().getOrDefault(\"{env_key}\", System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\")) + \"/fixtures/{fixture_id}\"));\n",
                                fixture_id = fixture.id,
                            ));
                            builder_expressions.push_str(&format!(
                                "        String {var_name}Json = {literal}.replace(\"{}\", {var_name}MockBaseUrl);\n",
                                crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                            ));
                            builder_expressions.push_str(&format!(
                                "        var {var_name} = JsonUtil.fromJson({var_name}Json, {opts_type}.class);\n",
                            ));
                        } else {
                            builder_expressions.push_str(&format!(
                                "        var {var_name} = JsonUtil.fromJson({literal}, {opts_type}.class);\n",
                            ));
                        }
                    } else if let Some(obj) = val.as_object() {
                        // Generate builder expression: TypeName.builder().withFieldName(value)...build()
                        let empty_path_fields: Vec<String> = Vec::new();
                        let path_fields = call_overrides.map(|o| &o.path_fields).unwrap_or(&empty_path_fields);
                        let builder_expr = java_builder_expression(
                            obj,
                            opts_type,
                            enum_fields,
                            nested_types,
                            nested_types_optional,
                            path_fields,
                            &ir_enum_map,
                        );
                        let var_name = &arg.name;
                        builder_expressions.push_str(&format!("        var {} = {};\n", var_name, builder_expr));
                    }
                }
            }
        }
    }

    let adapter_lookup_name = call_config.core_lookup_name(lang);
    let adapter_lookup_names: Vec<&str> = adapter_lookup_name.as_deref().into_iter().collect();
    let adapter = adapter_lookup_names
        .iter()
        .find_map(|name| adapters.iter().find(|a| a.name == *name));
    let adapter_request_type: Option<String> = adapter
        .and_then(|a| a.request_type.as_deref())
        .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());

    // Determine if this is a streaming adapter.
    let is_streaming_adapter =
        adapter.is_some_and(|a| matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming));

    // When a non-streaming adapter with owner_type is present, filter out handle-type args
    // since the facade method doesn't take them separately (the handle is
    // encapsulated in the adapter).
    let filtered_args: Vec<_> = if adapter.is_some_and(|a| a.owner_type.is_some()) && !is_streaming_adapter {
        args.iter().filter(|arg| arg.arg_type != "handle").cloned().collect()
    } else {
        args.to_vec()
    };

    // Streaming owner_type adapters are facade-exposed as INSTANCE methods on the
    // owner handle (`engine.streamItems(req)`), not as static facade methods — the
    // Java facade deliberately emits no static streaming methods. Capture the owner
    // handle variable so the call is rendered as an instance-method invocation.
    let streaming_owner_handle: Option<String> =
        if is_streaming_adapter && adapter.is_some_and(|a| a.owner_type.is_some()) {
            filtered_args
                .iter()
                .find(|a| a.arg_type == "handle")
                .map(|a| a.name.clone())
        } else {
            None
        };

    let mut teardown_block = String::new();
    let (mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        &filtered_args,
        JavaArgsContext {
            class_name,
            options_type: effective_options_type,
            fixture,
            adapter_request_type: adapter_request_type.as_deref(),
            owner_handle_is_receiver: streaming_owner_handle.is_some(),
            config,
            type_defs,
            enums,
            target_params,
            teardown_block: &mut teardown_block,
        },
    );

    // Per-language `extra_args` from call overrides — verbatim trailing
    // expressions appended after the configured args (e.g. `null` for an
    // optional trailing parameter the fixture cannot supply). Mirrors the
    // TypeScript and C# implementations.
    let extra_args_slice: &[String] = recipe.extra_args;

    let mut final_args = args_str;
    if let Some(visitor_spec) = &fixture.visitor {
        if let Some(binding) = java_visitor_binding(config, type_defs, Some(visitor_spec), effective_options_type) {
            // Generic discriminated-union result types are supported by the Jinja
            // template via the same factory shape as the default fallback type —
            // drop the historical bail-out and let the generated code compile or
            // surface a clear method-arity diagnostic from the host project's
            // binding.
            let visitor_var = build_java_visitor(&mut setup_lines, visitor_spec, class_name, &binding);
            final_args = apply_java_visitor_arg(&mut setup_lines, &final_args, args, &visitor_var, &binding);
        } else {
            setup_lines.push(format!(
                "org.junit.jupiter.api.Assumptions.assumeTrue(false, \"java visitor fixture '{}' requires trait_bridge options_type, options_field, context_type, and result_type metadata\");",
                escape_java(&fixture.id)
            ));
        }
    }

    if !extra_args_slice.is_empty() {
        let extra_str = extra_args_slice.join(", ");
        final_args = if final_args.is_empty() {
            extra_str
        } else {
            format!("{final_args}, {extra_str}")
        };
    }

    // Render assertions_body
    let mut assertions_body = String::new();

    // Emit a `source` variable for run_query assertions that need the raw bytes.
    let needs_source_var = fixture
        .assertions
        .iter()
        .any(|a| a.assertion_type == "method_result" && a.method.as_deref() == Some("run_query"));
    if needs_source_var && let Some(source_arg) = args.iter().find(|a| a.field == "source_code") {
        let field = source_arg.field.strip_prefix("input.").unwrap_or(&source_arg.field);
        if let Some(val) = fixture.input.get(field) {
            let java_val = json_to_java(val);
            assertions_body.push_str(&format!("        var source = {}.getBytes();\n", java_val));
        }
    }

    // Merge per-call java enum_fields with the file-level java enum_fields so that
    // call-specific enum-typed result fields (e.g. `choices[0].finish_reason` for
    // chat) trigger Optional<Enum> coercion even when the global override block
    // does not list them. Per-call entries take precedence.
    // For assertions, use assert_enum_fields from the call override to get field->type mappings.
    // Build a HashMap that merges both for assertion handling.
    let assert_enum_types: std::collections::HashMap<String, String> = if let Some(co) = call_overrides {
        co.assert_enum_fields.clone()
    } else {
        std::collections::HashMap::new()
    };

    // Keep the old effective_enum_fields as a HashSet for backward compatibility with other code paths.
    let mut effective_enum_fields: std::collections::HashSet<String> = enum_fields.clone();
    if let Some(co) = call_overrides {
        for k in co.enum_fields.keys() {
            effective_enum_fields.insert(k.clone());
        }
    }

    // Streaming detection (call-level `streaming` opt-out is honored). Computed
    // here so `render_assertion` can suppress the streaming-virtual-field path
    // for non-streaming fixtures whose real result struct has a literal `chunks`
    // field that would otherwise collide with the virtual aggregator name.
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());
    let streaming_item_type =
        crate::e2e::codegen::recipe::streaming_item_type(call_config, adapters, &adapter_lookup_names);
    let fractional_fields = fractional_scalar_fields(type_defs);
    // WHETHER `not_error` may assert presence is decided once, centrally — see
    // `not_error_presence::may_assert_presence`'s doc for why a sibling assertion or an
    // `Option<T>` result both make an unconditional presence check unsafe. ~keep
    let not_error_may_assert_presence =
        crate::e2e::codegen::not_error_presence::may_assert_presence(fixture, effective_result_is_option);

    for assertion in &fixture.assertions {
        render_assertion(
            &mut assertions_body,
            assertion,
            result_var,
            class_name,
            field_resolver,
            effective_result_is_simple,
            effective_result_is_bytes,
            effective_result_is_option,
            is_streaming,
            streaming_item_type,
            &effective_enum_fields,
            &assert_enum_types,
            call_config.returns_void,
            &fractional_fields,
            not_error_may_assert_presence,
        );
        ensure_assertion_line_ending(&mut assertions_body);
    }
    crate::e2e::codegen::fail_on_unavailable_field_markers(&assertions_body, "java", &fixture.id, &fixture.assertions);
    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&assertions_body, "java", &fixture.id);
    // A `returns_void` call binds no `result_var`, so a fixture whose only assertion is
    // `not_error` has nothing to assert on the way non-void calls do. Wrap the call itself
    // in `assertDoesNotThrow` instead, so the check is a real, visible assertion rather than
    // a bare statement relying only on `throws_clause` to fail the test on an uncaught
    // exception. ~keep
    let void_not_error = call_config.returns_void && fixture.assertions.iter().any(|a| a.assertion_type == "not_error");
    // ~keep `expects_error` is excluded because `test_method.jinja` does not splice
    // `assertions_body` into that branch at all — the assertThrows IS the expectation, and the
    // dropped assertions beside it are already surfaced by `error_path_assertions::render` below.
    // Refusing there would replace a real check with a skip. `void_not_error` is excluded for the
    // same reason: its real assertion is `assertDoesNotThrow(() -> call_expr)` wrapped around the
    // call itself below, not anything spliced into `assertions_body` — `inert_verdict` only sees
    // `assertions_body` and would otherwise misread a correctly-empty body as vacuous and replace
    // it with a skip, discarding the real check that already exists one line down.
    let verdict = if expects_error || void_not_error {
        None
    } else {
        inert_example::inert_verdict(&assertions_body, "java", &fixture.id, &fixture.assertions)
    };
    if let Some(refusal) = verdict {
        inert_example::record_refusal(&refusal);
        assertions_body = render_java_refusal(&assertions_body, &refusal);
    }

    let throws_clause = " throws Exception";

    // When client_factory is set, instantiate a client and dispatch the call as
    // a method on the client; otherwise call the static helper on `class_name`.
    let (client_setup_lines, call_target) = if let Some(factory) = client_factory.as_deref() {
        let factory_name = factory.to_lower_camel_case();
        let fixture_id = &fixture.id;
        let mut setup: Vec<String> = Vec::new();
        let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
        let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
        if let Some(var) = api_key_var.filter(|_| has_mock) {
            setup.push(format!("String apiKey = System.getenv(\"{var}\");"));
            setup.push(format!(
                "String mockServerUrl = System.getProperty(\"mockServerUrl\"); if (mockServerUrl == null) {{ mockServerUrl = System.getenv(\"MOCK_SERVER_URL\"); }} String baseUrl = (apiKey != null && !apiKey.isEmpty()) ? null : (mockServerUrl != null ? mockServerUrl + \"/fixtures/{fixture_id}\" : \"http://localhost:8000/fixtures/{fixture_id}\");"
            ));
            setup.push(format!(
                "System.out.println(\"{fixture_id}: \" + (baseUrl == null ? \"using real API ({var} is set)\" : \"using mock server ({var} not set)\"));"
            ));
            setup.push(format!(
                "var client = {class_name}.{factory_name}(baseUrl == null ? apiKey : \"test-key\", baseUrl, null, null, null);"
            ));
        } else if has_mock {
            if fixture.has_host_root_route() {
                setup.push(format!(
                    "String mockServerUrl = System.getProperty(\"mockServerUrl\"); if (mockServerUrl == null) {{ mockServerUrl = System.getenv(\"MOCK_SERVER_URL\"); }} String defaultUrl = (mockServerUrl != null ? mockServerUrl : \"http://localhost:8000\") + \"/fixtures/{fixture_id}\"; String mockUrl = System.getProperty(\"mockServer.{fixture_id}\", defaultUrl);"
                ));
            } else {
                setup.push(format!(
                    "String mockServerUrl = System.getProperty(\"mockServerUrl\"); if (mockServerUrl == null) {{ mockServerUrl = System.getenv(\"MOCK_SERVER_URL\"); }} String mockUrl = (mockServerUrl != null ? mockServerUrl : \"http://localhost:8000\") + \"/fixtures/{fixture_id}\";"
                ));
            }
            setup.push(format!(
                "var client = {class_name}.{factory_name}(\"test-key\", mockUrl, null, null, null);"
            ));
        } else if let Some(api_key_var) = api_key_var {
            setup.push(format!("String apiKey = System.getenv(\"{api_key_var}\");"));
            setup.push(format!(
                "org.junit.jupiter.api.Assumptions.assumeTrue(apiKey != null && !apiKey.isEmpty(), \"{api_key_var} not set\");"
            ));
            setup.push(format!("var client = {class_name}.{factory_name}(apiKey);"));
        } else {
            setup.push(format!("var client = {class_name}.{factory_name}(\"test-key\");"));
        }
        (setup, "client".to_string())
    } else {
        (Vec::new(), class_name.to_string())
    };

    // Prepend client setup before any other setup_lines.
    let combined_setup: Vec<String> = client_setup_lines.into_iter().chain(setup_lines).collect();

    let call_expr = if let Some(ref handle_var) = streaming_owner_handle {
        // Instance-method invocation on the owner handle.
        format!("{handle_var}.{function_name}({final_args})")
    } else {
        format!("{call_target}.{function_name}({final_args})")
    };

    // `is_streaming` was computed earlier (before the assertion render loop).
    let collect_snippet = if is_streaming && !expects_error {
        // Derive the item_type from the adapter if present; otherwise use the default.
        crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet_typed(
            "java",
            result_var,
            "chunks",
            streaming_item_type,
        )
        .unwrap_or_default()
    } else {
        String::new()
    };

    let declared_error_check = declared_error_value_check(fixture, errors);
    // ~keep The `expects_error` branch of `java/test_method.jinja` renders the assertThrows and
    // nothing else, so every other assertion on an error fixture — most often an `equals` against
    // `error.status_code` — used to leave no trace at all in the generated test.
    let unrenderable_error_assertions =
        crate::e2e::codegen::error_path_assertions::render(fixture, "        // ", "java");

    let rendered = crate::e2e::template_env::render(
        "java/test_method.jinja",
        crate::alef_context! {
            method_name => method_name,
            description => description,
            builder_expressions => builder_expressions,
            setup_lines => combined_setup,
            throws_clause => throws_clause,
            expects_error => expects_error,
            declared_error_check => declared_error_check,
            unrenderable_error_assertions => unrenderable_error_assertions.trim_end(),
            call_expr => call_expr,
            result_var => result_var,
            returns_void => call_config.returns_void,
            void_not_error => void_not_error,
            collect_snippet => collect_snippet,
            assertions_body => assertions_body,
            teardown_block => teardown_block,
        },
    );
    out.push_str(&rendered);
}

#[cfg(test)]
mod declared_error_value_check_tests {
    use super::declared_error_value_check;
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

    fn error_def_with(variant_name: &str, error_code: Option<u32>) -> Vec<ErrorDef> {
        vec![ErrorDef {
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: variant_name.to_string(),
                error_code,
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }]
    }

    #[test]
    fn no_declared_value_produces_no_check() {
        let fixture = Fixture::default();
        assert_eq!(declared_error_value_check(&fixture, &[]), None);
    }

    /// No `errors` IR supplied: a value cannot be recognised as a known variant name, so it
    /// renders exactly like a message-style value always did before this fix.
    #[test]
    fn declared_value_checks_message_or_class_name() {
        let fixture = fixture_with_declared_error("BadRequest");
        let check = declared_error_value_check(&fixture, &[]).expect("expected a rendered check");
        assert!(
            check.contains("thrown.getMessage() != null && thrown.getMessage().contains(\"BadRequest\")"),
            "got: {check}"
        );
        assert!(
            check.contains("thrown.getClass().getSimpleName().contains(\"BadRequest\")"),
            "got: {check}"
        );
    }

    #[test]
    fn declared_value_with_quotes_and_backslashes_is_escaped() {
        let fixture = fixture_with_declared_error("bad \"field\" \\ value");
        let check = declared_error_value_check(&fixture, &[]).expect("expected a rendered check");
        assert!(
            check.contains("bad \\\"field\\\" \\\\ value"),
            "expected escaped literal, got: {check}"
        );
    }

    /// Java IS substantiable when the variant declared `#[alef(error_code = N)]`: the assertion
    /// still renders as before.
    #[test]
    fn a_coded_known_variant_still_asserts() {
        let fixture = fixture_with_declared_error("Authentication");
        let errors = error_def_with("Authentication", Some(100));
        let check = declared_error_value_check(&fixture, &errors).expect("expected a rendered check");
        assert!(check.contains("assertTrue("), "got: {check}");
        assert!(check.contains("\"Authentication\""), "got: {check}");
    }

    /// The defect this fix closes: a declared value naming a real `ErrorVariant` with no
    /// `error_code` must render the registered skip, not an assertion that can never pass.
    #[test]
    fn an_uncoded_known_variant_renders_the_skip() {
        let fixture = fixture_with_declared_error("Authentication");
        let errors = error_def_with("Authentication", None);
        let check = declared_error_value_check(&fixture, &errors).expect("expected a rendered skip");
        assert_eq!(
            check,
            "        // skipped: declared error variant 'Authentication' not substantiated by this backend's \
             generated error type"
        );
        assert!(
            !check.contains("assertTrue"),
            "must not render an assertion, got: {check}"
        );
    }
}

#[cfg(test)]
mod assertion_line_ending_tests {
    use super::ensure_assertion_line_ending;

    #[test]
    fn separates_consecutive_rendered_assertions() {
        let mut assertions = "        assertTrue(first);".to_string();
        ensure_assertion_line_ending(&mut assertions);
        assertions.push_str("        assertTrue(second);");
        ensure_assertion_line_ending(&mut assertions);

        assert_eq!(assertions, "        assertTrue(first);\n        assertTrue(second);\n");
    }
}

#[cfg(test)]
mod dropped_field_marker_tests {
    use super::render_test_method;
    use crate::e2e::config::{CallConfig, E2eConfig};
    use crate::e2e::fixture::{Assertion, Fixture};
    use std::collections::HashSet;

    fn make_fixture(id: &str, field: &str) -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: id.to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
            assertions: vec![Assertion {
                assertion_type: "equals".to_string(),
                field: Some(field.to_string()),
                value: Some(serde_json::json!("x")),
                ..Default::default()
            }],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        }
    }

    /// Regression test for alef task #81: Java's "skipped: field not available" comment
    /// must carry the exact marker text the shared `fail_on_unavailable_field_markers`
    /// mechanism (src/e2e/codegen/mod.rs) matches on, so arming
    /// `ALEF_E2E_STRICT_FIELD_AVAILABILITY` turns a dropped field assertion into a
    /// generation-time failure. The arming behaviour itself is proven in `mod.rs`'s
    /// `unavailable_field_marker_tests`; this test only pins the marker text Java emits
    /// through the real per-fixture rendering entry point. ~keep
    #[test]
    fn dropped_field_assertion_carries_the_marker_that_arms_the_strict_mode() {
        let fixture = make_fixture("process_smoke", "nonexistent_field");
        let call = CallConfig {
            function: "process".to_string(),
            module: "MyLib".to_string(),
            result_var: "result".to_string(),
            result_fields: HashSet::from(["content".to_string()]),
            returns_result: true,
            ..Default::default()
        };
        let e2e_config = E2eConfig {
            call,
            ..Default::default()
        };
        let config = crate::core::config::ResolvedCrateConfig::default();
        let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();

        let mut out = String::new();
        render_test_method(
            &mut out,
            &fixture,
            "SampleClass",
            "",
            "",
            &[],
            None,
            false,
            &e2e_config,
            &std::collections::HashMap::new(),
            false,
            &[],
            &config,
            &type_defs,
            &[],
            &[],
            &[],
        );

        assert!(
            out.contains("field 'nonexistent_field' not available on result type"),
            "got:\n{out}"
        );
    }

    fn render_java_error_method(extra: Vec<Assertion>, errors: &[crate::core::ir::ErrorDef]) -> String {
        let mut fixture = make_fixture("rate_limited", "content");
        fixture.assertions = vec![Assertion {
            assertion_type: "error".to_string(),
            ..Default::default()
        }];
        fixture.assertions.extend(extra);
        let e2e_config = E2eConfig {
            call: CallConfig {
                function: "process".to_string(),
                module: "MyLib".to_string(),
                result_var: "result".to_string(),
                returns_result: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let config = crate::core::config::ResolvedCrateConfig::default();
        let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();

        let mut out = String::new();
        let _ = crate::e2e::codegen::take_skip_records();
        render_test_method(
            &mut out,
            &fixture,
            "SampleClass",
            "",
            "",
            &[],
            None,
            false,
            &e2e_config,
            &std::collections::HashMap::new(),
            false,
            &[],
            &config,
            &type_defs,
            // Error-path fixture: no enums and no IR functions, i.e. the IrAbsent state the
            // typed-argument seam must leave untouched. ~keep
            &[],
            &[],
            errors,
        );
        out
    }

    /// Java's `expects_error` branch renders `assertThrows(..)` and returns, so every other
    /// assertion on the fixture used to leave no trace in the generated test at all.
    #[test]
    fn java_equals_on_an_error_field_is_named_instead_of_dropped() {
        let out = render_java_error_method(
            vec![Assertion {
                assertion_type: "equals".to_string(),
                field: Some("error.status_code".to_string()),
                ..Default::default()
            }],
            &[],
        );

        // Positive first: the error block really rendered.
        assert!(
            out.contains("assertThrows(Exception.class, () -> {"),
            "the error block must render:\n{out}"
        );
        assert!(
            out.contains(
                "// skipped: assertion type 'equals' has no accessor for error field error.status_code in this \
                 backend"
            ),
            "got:\n{out}"
        );

        let records = crate::e2e::codegen::take_skip_records();
        assert_eq!(records.len(), 1, "got: {records:?}");
        assert_eq!(records[0].language, "java");
        assert_eq!(records[0].field, "equals");
    }

    /// Negative control: a lone `error` assertion must leave the generated method marker-free.
    #[test]
    fn java_a_lone_error_assertion_renders_no_marker() {
        let out = render_java_error_method(Vec::new(), &[]);

        assert!(
            out.contains("assertThrows(Exception.class, () -> {"),
            "the error block must render:\n{out}"
        );
        assert!(!out.contains("has no accessor for error field"), "got:\n{out}");
    }

    /// End-to-end proof through the real `render_test_method` entry point: a fixture whose
    /// declared `error` value names a real, UNCODED `ErrorVariant` used to render an
    /// `assertTrue(...)` that can never pass (the measured consumer defect). It must now render
    /// the registered skip instead.
    #[test]
    fn declared_variant_with_no_error_code_renders_the_skip_end_to_end() {
        use crate::core::ir::{ErrorDef, ErrorVariant};

        let mut fixture = make_fixture("auth_fails", "content");
        fixture.assertions = vec![Assertion {
            assertion_type: "error".to_string(),
            value: Some(serde_json::Value::String("Authentication".to_string())),
            ..Default::default()
        }];
        let e2e_config = E2eConfig {
            call: CallConfig {
                function: "process".to_string(),
                module: "MyLib".to_string(),
                result_var: "result".to_string(),
                returns_result: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let config = crate::core::config::ResolvedCrateConfig::default();
        let errors = vec![ErrorDef {
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Authentication".to_string(),
                error_code: None,
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }];

        let mut out = String::new();
        render_test_method(
            &mut out,
            &fixture,
            "SampleClass",
            "",
            "",
            &[],
            None,
            false,
            &e2e_config,
            &std::collections::HashMap::new(),
            false,
            &[],
            &config,
            &[],
            &[],
            &[],
            &errors,
        );

        assert!(
            out.contains(
                "// skipped: declared error variant 'Authentication' not substantiated by this backend's \
                 generated error type"
            ),
            "got:\n{out}"
        );
        assert!(
            !out.contains("assertTrue(thrown.getMessage()"),
            "must not render an assertion that can never pass, got:\n{out}"
        );
    }
}

#[cfg(test)]
mod inert_example_refusal_tests;
