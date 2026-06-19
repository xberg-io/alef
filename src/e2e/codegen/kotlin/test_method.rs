use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;

use super::args::{KotlinArgsContext, build_args_and_setup};
use super::assertions::render_assertion;

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
    type_enum_fields: &std::collections::HashMap<String, HashSet<String>>,
    kotlin_android_style: bool,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) {
    // Delegate HTTP fixtures to the HTTP-specific renderer.
    if let Some(http) = &fixture.http {
        super::http::render_http_test_method(out, fixture, http);
        return;
    }

    // Resolve per-fixture call config (supports named calls via fixture.call field).
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Build per-call field resolver using the effective field sets for this call.
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &HashSet::new(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone());
    let field_resolver = &call_field_resolver;
    let enum_fields = e2e_config.effective_fields_enum(call_config);
    let lang = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };
    let call_overrides = call_config.overrides.get(lang);

    // Check for client_factory — when set, use instance-method call style.
    // Falls back to the global `[e2e.call.overrides.kotlin]` `client_factory` when
    // a per-call override is absent, matching the dart/swift renderers.
    //
    // For `kotlin_android_style`, also check `kotlin_android` and then `java`
    // overrides when neither a `kotlin` per-call nor a `kotlin` global override
    // is present. kotlin_android shares the same JNI bridge entry-points as the
    // Java facade, so a `java` `client_factory` applies equally.
    let client_factory = call_overrides
        .and_then(|o| o.client_factory.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get(lang)
                .and_then(|o| o.client_factory.as_deref())
        })
        .or_else(|| {
            if !kotlin_android_style {
                return None;
            }
            // kotlin_android fallback: check per-call kotlin_android → java, then
            // global kotlin_android → java overrides.
            call_config
                .overrides
                .get("kotlin_android")
                .and_then(|o| o.client_factory.as_deref())
                .or_else(|| {
                    call_config
                        .overrides
                        .get("java")
                        .and_then(|o| o.client_factory.as_deref())
                })
                .or_else(|| {
                    e2e_config
                        .call
                        .overrides
                        .get("kotlin_android")
                        .and_then(|o| o.client_factory.as_deref())
                })
                .or_else(|| {
                    e2e_config
                        .call
                        .overrides
                        .get("java")
                        .and_then(|o| o.client_factory.as_deref())
                })
        });

    let effective_function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.to_lower_camel_case());
    // Resolve per-fixture class name: prefer the kotlin_android call override, then
    // fall back to the global class_name. For trait bridge calls like register_document_extractor,
    // the override specifies the bridge class (e.g., DocumentExtractorBridge).
    let effective_class_name = call_overrides
        .and_then(|o| o.class.as_ref())
        .cloned()
        .or_else(|| {
            // For kotlin_android, also check the kotlin_android override.
            if kotlin_android_style {
                call_config
                    .overrides
                    .get("kotlin_android")
                    .and_then(|o| o.class.as_ref())
                    .cloned()
            } else {
                None
            }
        })
        .unwrap_or_else(|| class_name.to_string());
    let effective_result_var = &call_config.result_var;
    let function_name = effective_function_name.as_str();
    let class_name_for_call = effective_class_name.as_str();
    let result_var = effective_result_var.as_str();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs);
    let args: &[crate::e2e::config::ArgMapping] = recipe.args;
    // Resolve per-fixture options_type using per-fixture call config resolution.
    // Per-fixture kotlin overrides take precedence, then fall back to class-level,
    // then to any other language's options_type for the same call (kotlin_android, java, csharp, etc.).
    // This mirrors the Python e2e codegen pattern where fixture_opts_type is resolved
    // per-fixture from the call config overrides, ensuring enums and types are correctly
    // imported and constructed.
    let compatible_options_languages: &[&str] = if kotlin_android_style {
        &["kotlin_android", "kotlin", "java", "csharp", "c", "go", "php", "python"]
    } else {
        &["csharp", "c", "go", "php", "python"]
    };
    let fixture_options_type: Option<String> = call_overrides
        .and_then(|o| o.options_type.clone())
        .or_else(|| options_type.map(str::to_string))
        .or_else(|| {
            recipe
                .compatible_options_type(compatible_options_languages)
                .map(str::to_string)
        });
    let options_type = fixture_options_type.as_deref();

    // Resolve per-fixture result_is_simple: prefer the kotlin override, then the
    // class-level default, then any sibling language override (java/csharp/go).
    // The Kotlin facade shares its return-type shape with the Java facade, so a
    // declaration in any of those bindings applies to Kotlin too.
    let effective_result_is_simple = call_overrides.is_some_and(|o| o.result_is_simple)
        || call_config.result_is_simple
        || result_is_simple
        || ["java", "csharp", "go"]
            .iter()
            .any(|cand| call_config.overrides.get(*cand).is_some_and(|o| o.result_is_simple));
    let result_is_simple = effective_result_is_simple;

    // Resolve per-fixture result_is_option: prefer the kotlin override, then the
    // call-level default. When set the function returns `T?` and bare-result
    // emptiness assertions must use a null-check instead of `.isEmpty()`.
    let result_is_option = call_overrides.is_some_and(|o| o.result_is_option) || call_config.result_is_option;

    let method_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Streaming detection (call-level `streaming` opt-out is honored).
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());
    let stream_lang = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };
    let collect_snippet = if is_streaming && !expects_error {
        crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet(
            stream_lang,
            result_var,
            "chunks",
        )
        .unwrap_or_default()
    } else {
        String::new()
    };

    // Check if this test needs ObjectMapper deserialization for json_object args.
    // Uses `resolve_field` so that `field = "input"` resolves to the whole fixture
    // input (and not a nested key called "input"), matching dart/swift behavior.
    // Also include tests with array element types, which are deserialized inline.
    let needs_deser = (options_type.is_some()
        && args.iter().any(|arg| {
            arg.arg_type == "json_object" && !crate::e2e::codegen::resolve_field(&fixture.input, &arg.field).is_null()
        }))
        || args.iter().any(|arg| {
            arg.arg_type == "json_object"
                && arg.element_type.is_some()
                && !crate::e2e::codegen::resolve_field(&fixture.input, &arg.field).is_null()
        });

    // Merge per-call kotlin enum_fields (HashMap key = field path, value = enum type name)
    // into the global fields_enum set so that call-specific enum-typed result fields
    // (e.g. `status` on BatchObject) route through `.getValue()` in assertions even
    // when absent from the global `fields_enum` list.  Mirrors the Java codegen at
    // codegen/java.rs where per-call overrides are merged before assertion rendering.
    //
    // Additionally, auto-detect enum-typed fields by looking up the call's result type
    // in `type_enum_fields` (built from the IR TypeDef list). This handles the common
    // case where a field's Rust type is a `Named(EnumName)` that was never explicitly
    // listed in the alef.toml `enum_fields` table.
    let effective_enum_fields: std::borrow::Cow<HashSet<String>> = {
        // Resolve the result type name for this call. Prefer the kotlin override, then
        // java, then c — the Kotlin facade re-exports Java facade types unchanged.
        let result_type_name: Option<&str> = call_overrides
            .and_then(|co| co.result_type.as_deref())
            .or_else(|| call_config.overrides.get("java").and_then(|o| o.result_type.as_deref()))
            .or_else(|| call_config.overrides.get("c").and_then(|o| o.result_type.as_deref()));
        let auto_enum_fields: Option<&HashSet<String>> = result_type_name.and_then(|name| type_enum_fields.get(name));
        // For kotlin_android, also pull enum_fields from the `java` and
        // `kotlin_android` per-call overrides, since those binding layers share
        // the same JNI bridge and response types.
        let java_call_overrides = if kotlin_android_style {
            call_config
                .overrides
                .get("java")
                .or_else(|| call_config.overrides.get("kotlin_android"))
        } else {
            None
        };
        let has_per_call = call_overrides.is_some_and(|co| !co.enum_fields.is_empty())
            || java_call_overrides.is_some_and(|co| !co.enum_fields.is_empty());
        let has_auto = auto_enum_fields.is_some_and(|f| !f.is_empty());
        if has_per_call || has_auto {
            let mut merged = enum_fields.clone();
            if let Some(co) = call_overrides {
                merged.extend(co.enum_fields.keys().cloned());
            }
            if let Some(co) = java_call_overrides {
                merged.extend(co.enum_fields.keys().cloned());
            }
            if let Some(auto_fields) = auto_enum_fields {
                merged.extend(auto_fields.iter().cloned());
            }
            std::borrow::Cow::Owned(merged)
        } else {
            std::borrow::Cow::Borrowed(enum_fields)
        }
    };
    let enum_fields: &HashSet<String> = &effective_enum_fields;

    let _ = writeln!(out, "    @Test");
    if client_factory.is_some() || kotlin_android_style {
        let _ = writeln!(out, "    fun test{method_name}() = runBlocking {{");
    } else {
        let _ = writeln!(out, "    fun test{method_name}() {{");
    }
    let _ = writeln!(out, "        // {description}");

    // Collect ObjectMapper deserialization bindings for json_object args.
    // Object args use the configured `options_type`. Array args carrying
    // `element_type` are emitted as inline List<T> literals below
    // (build_args_and_setup), so no deser binding is needed.
    //
    // For error tests we want these `val xxx = MAPPER.readValue(...)` lines
    // INSIDE the assertFailsWith block, so that Jackson validation errors on
    // the request literal (e.g. an unknown enum like `purpose: "invalid"`)
    // are caught by the test instead of bubbling up as test failures. So
    // collect into a Vec and let the caller decide where to emit them.
    let mut deser_lines: Vec<String> = Vec::new();
    if needs_deser {
        for arg in args {
            if arg.arg_type != "json_object" {
                continue;
            }
            let val = crate::e2e::codegen::resolve_field(&fixture.input, &arg.field);
            if val.is_null() {
                continue;
            }
            // Skip arrays that we materialise inline rather than deserialising via Jackson.
            if val.is_array() && arg.element_type.is_some() {
                continue;
            }
            let Some(opts_type) = options_type else { continue };
            let normalized = crate::e2e::codegen::transform_json_keys_for_language(val, "snake_case");
            let json_str = serde_json::to_string(&normalized).unwrap_or_default();
            let var_name = &arg.name;
            deser_lines.push(format!(
                "val {var_name} = MAPPER.readValue(\"{}\", {opts_type}::class.java)",
                crate::e2e::escape::escape_kotlin(&json_str)
            ));
        }
    }
    if !expects_error {
        for line in &deser_lines {
            let _ = writeln!(out, "        {line}");
        }
    }

    let (setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        KotlinArgsContext {
            fixture,
            class_name,
            options_type,
            fixture_id: &fixture.id,
            kotlin_android_style,
            config,
            type_defs,
        },
    );

    // When client_factory is set, emit client-object instantiation + instance method call.
    // The factory name is a function on the Kotlin facade object
    // that constructs the coroutine-friendly Kotlin client wrapper from the
    // raw apiKey + baseUrl pair the test owns.
    if let Some(factory) = client_factory {
        let fixture_id = &fixture.id;
        // Prefer system properties set by MockServerListener (which spawns the
        // mock-server in-process when MOCK_SERVER_URL isn't pre-set). The
        // per-fixture property holds the full URL; fall back to the base URL
        // (mockServerUrl or env var) with the /fixtures/<id> suffix appended.
        let mock_url_expr = format!(
            "System.getProperty(\"mockServer.{fixture_id}\", (System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\") ?: \"\") ?: \"\") + \"/fixtures/{fixture_id}\")"
        );
        if expects_error {
            // Wrap setup + client construction + call in assertFailsWith so
            // validation errors thrown during request construction
            // (e.g. Jackson rejecting an unknown enum literal) are also caught.
            // Mirrors the flat-function path's behaviour below.
            // For streaming functions, the error fires during collection, not at
            // call time — append the collect suffix so the flow is consumed.
            let call_expr = if is_streaming {
                let collect_suffix = if kotlin_android_style {
                    ".toList()"
                } else {
                    ".asSequence().toList()"
                };
                format!("client.{function_name}({args_str}){collect_suffix}")
            } else {
                format!("client.{function_name}({args_str})")
            };
            let _ = writeln!(out, "        assertFailsWith<Exception> {{");
            for line in &deser_lines {
                let _ = writeln!(out, "            {line}");
            }
            for line in &setup_lines {
                let _ = writeln!(out, "            {line}");
            }
            let _ = writeln!(
                out,
                "            val client = {class_name_for_call}.{factory}(apiKey = \"test-key\", baseUrl = {mock_url_expr})"
            );
            let _ = writeln!(out, "            {call_expr}");
            let _ = writeln!(out, "            client.close()");
            let _ = writeln!(out, "        }}");
            // Trailing `Unit` so the runBlocking { ... } lambda's final
            // expression is Unit (not the Exception returned by assertFailsWith).
            // The enclosing `fun ... = runBlocking { ... }` then infers Unit
            // as the test function's return type — JUnit 5 silently skips
            // any @Test method whose return type is not void/Unit.
            let _ = writeln!(out, "        Unit");
            let _ = writeln!(out, "    }}");
            return;
        }
        for line in &setup_lines {
            let _ = writeln!(out, "        {line}");
        }
        let _ = writeln!(
            out,
            "        val client = {class_name_for_call}.{factory}(apiKey = \"test-key\", baseUrl = {mock_url_expr})"
        );
        let _ = writeln!(out, "        val {result_var} = client.{function_name}({args_str})");
        if !collect_snippet.is_empty() {
            let _ = writeln!(out, "        {collect_snippet}");
        }
        for assertion in &fixture.assertions {
            render_assertion(
                out,
                assertion,
                result_var,
                class_name,
                field_resolver,
                result_is_simple,
                result_is_option,
                enum_fields,
                e2e_config.effective_fields_c_types(call_config),
                is_streaming,
                kotlin_android_style,
            );
        }
        let _ = writeln!(out, "        client.close()");
        let _ = writeln!(out, "    }}");
        return;
    }

    // Flat-function call style (no client_factory).
    if expects_error {
        // Wrap setup + call in assertFailsWith so validation errors thrown
        // during engine creation are also caught (mirrors Java's assertThrows).
        let _ = writeln!(out, "        assertFailsWith<Exception> {{");
        for line in &deser_lines {
            let _ = writeln!(out, "            {line}");
        }
        for line in &setup_lines {
            let _ = writeln!(out, "            {line}");
        }
        let _ = writeln!(out, "            {class_name_for_call}.{function_name}({args_str})");
        let _ = writeln!(out, "        }}");
        // Trailing Unit — see comment in the client-factory branch above.
        let _ = writeln!(out, "        Unit");
        let _ = writeln!(out, "    }}");
        return;
    }

    for line in &setup_lines {
        let _ = writeln!(out, "        {line}");
    }

    let _ = writeln!(
        out,
        "        val {result_var} = {class_name_for_call}.{function_name}({args_str})"
    );

    if !collect_snippet.is_empty() {
        let _ = writeln!(out, "        {collect_snippet}");
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            class_name,
            field_resolver,
            result_is_simple,
            result_is_option,
            enum_fields,
            &e2e_config.fields_c_types,
            is_streaming,
            kotlin_android_style,
        );
    }

    let _ = writeln!(out, "    }}");
}
