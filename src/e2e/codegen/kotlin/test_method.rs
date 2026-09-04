//! Kotlin test-method rendering.
//!
//! ~keep This file is already over the repo's 1,000-line file-modularization cap. The
//! `not_error_may_assert_presence` unification (routing `not_error` through
//! `not_error_presence::may_assert_presence`) added one call-site argument to both
//! `render_assertion` invocations here plus the shared computation feeding them — a small,
//! bounded amount of production wiring, not new unrelated functionality.

mod inert_refusal;
use inert_refusal::refuse_inert_example;

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;

use super::args::{KotlinArgsContext, build_args_and_setup};
use super::assertions::render_assertion;
use crate::e2e::escape::escape_kotlin;

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
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> anyhow::Result<()> {
    // Delegate HTTP fixtures to the HTTP-specific renderer.
    if let Some(http) = &fixture.http {
        super::http::render_http_test_method(out, fixture, http);
        return Ok(());
    }

    // Resolve per-fixture call config (supports named calls via fixture.call field).
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let lang = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };
    // Build per-call field resolver using the effective field sets for this call. Extracted to
    // `call_field_resolver.rs` (this file is at the file-size ratchet's frozen ceiling).
    let call_field_resolver = super::call_field_resolver::build_call_field_resolver(
        e2e_config,
        call_config,
        fixture,
        lang,
        type_defs,
        enums,
        functions,
    );
    let field_resolver = &call_field_resolver;
    let enum_fields = e2e_config.effective_fields_enum(call_config);
    let json_scalar_fields = e2e_config.effective_fields_json_scalar(call_config);
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
    let function_name = effective_function_name.as_str();
    let class_name_for_call = effective_class_name.as_str();
    let result_var = call_config.effective_result_var();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs)
        .with_functions(functions);
    let args: &[crate::e2e::config::ArgMapping] = recipe.args;
    let target_params = recipe.target_params(lang);
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
    // WHETHER `not_error` may assert presence is decided once, centrally — see
    // `not_error_presence::may_assert_presence`'s doc for why a sibling assertion or an
    // `Option<T>` result both make an unconditional presence check unsafe. ~keep
    let not_error_may_assert_presence =
        crate::e2e::codegen::not_error_presence::may_assert_presence(fixture, result_is_option);
    let adapter_lookup_name = call_config.core_lookup_name(lang);
    let adapter = adapter_lookup_name
        .as_deref()
        .and_then(|name| config.adapters.iter().find(|adapter| adapter.name == name));
    let streaming_request = adapter.and_then(|adapter| {
        matches!(adapter.pattern, crate::core::config::extras::AdapterPattern::Streaming)
            .then(|| adapter.params.first())
            .flatten()
    });

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

    // Streaming owner_type adapters are facade-exposed as INSTANCE methods on the
    // owner handle (`engine.streamItems(req)`), not as static/client facade
    // methods — mirrors the Java e2e backend (java/test_method.rs), which
    // deliberately emits no static/client streaming methods for these adapters.
    // Capture the owner handle variable so the call is rendered as an
    // instance-method invocation instead of falling through to the flat-function
    // (or client-factory) call style.
    let is_streaming_owner_adapter = adapter.is_some_and(|a| {
        matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming) && a.owner_type.is_some()
    });
    let streaming_owner_handle: Option<String> = if is_streaming_owner_adapter {
        args.iter().find(|a| a.arg_type == "handle").map(|a| a.name.clone())
    } else {
        None
    };

    let _ = writeln!(out, "    @Test");
    if client_factory.is_some() || kotlin_android_style {
        // `: Unit` is load-bearing, not style. An expression-bodied `fun x() = runBlocking
        // { ... }` takes its return type from the lambda's final expression, and JUnit 5 does
        // not execute a `@Test` whose return type is not void -- silently, with the suite still
        // reporting success. `kotlin.test.assertNotNull` (and every other assertion that hands
        // back the value it checked) therefore used to make the enclosing test non-Unit and
        // unrunnable: in one consumer's kotlin-android suite 18 of 97 generated tests never ran,
        // one class losing all 5. Declaring the type here constrains every branch at once and lets Kotlin
        // coerce the lambda result, where a trailing `Unit` statement only covers the one exit
        // path it sits on. ~keep
        let _ = writeln!(out, "    fun test{method_name}(): Unit = runBlocking {{");
    } else {
        let _ = writeln!(out, "    fun test{method_name}() {{");
    }
    let _ = writeln!(out, "        // {description}");

    // ObjectMapper deserialization bindings for json_object args (`val xxx =
    // MAPPER.readValue(...)`) are owned entirely by `build_args_and_setup`
    // (see args.rs), which pushes them onto `setup_lines` below. Do not
    // duplicate that here — see the `setup_lines` doc comment on
    // `build_args_and_setup` for why it must be the sole emitter.

    let call_args: Vec<_> = if streaming_owner_handle.is_some() && streaming_request.is_some() {
        args.iter().filter(|arg| arg.arg_type == "handle").cloned().collect()
    } else {
        args.to_vec()
    };
    let (mut setup_lines, mut args_str) = build_args_and_setup(
        &fixture.input,
        &call_args,
        KotlinArgsContext {
            fixture,
            class_name,
            options_type,
            fixture_id: &fixture.id,
            kotlin_android_style,
            config,
            type_defs,
            enums,
            owner_handle_is_receiver: streaming_owner_handle.is_some(),
            target_params,
        },
    )?;
    if streaming_owner_handle.is_some()
        && let Some(request) = streaming_request
    {
        let request_name = request.name.to_lower_camel_case();
        let request_type = request.ty.rsplit("::").next().unwrap_or(&request.ty);
        let mut request_input = fixture.input.clone();
        if let Some(object) = request_input.as_object_mut() {
            for handle in args.iter().filter(|arg| arg.arg_type == "handle") {
                let field = handle.field.strip_prefix("input.").unwrap_or(&handle.field);
                object.remove(field);
            }
        }
        let normalized = crate::e2e::codegen::transform_json_keys_for_language(&request_input, "snake_case");
        let request_json = serde_json::to_string(&normalized).unwrap_or_default();
        // A full literal expression -- quoted, or `+`-chunked when `request_json` is long
        // enough to threaten the JVM's 65535-byte constant cap -- not just escaped content.
        // See `values::kotlin_string_literal`.
        let literal = super::values::kotlin_string_literal(&request_json);
        if crate::e2e::codegen::value_contains_mock_url_placeholder(&normalized) {
            let env_key = crate::e2e::codegen::mock_url_env_key(&fixture.id);
            setup_lines.push(format!(
                "val {request_name}Json = {literal}.replace(\"{}\", System.getProperty(\"mockServer.{}\", System.getenv(\"{env_key}\") ?: \"\"))",
                crate::e2e::escape::escape_kotlin(crate::e2e::codegen::MOCK_URL_PLACEHOLDER), fixture.id,
            ));
            setup_lines.push(format!(
                "val {request_name} = MAPPER.readValue({request_name}Json, {request_type}::class.java)"
            ));
        } else {
            setup_lines.push(format!(
                "val {request_name} = MAPPER.readValue({literal}, {request_type}::class.java)"
            ));
        }
        args_str = request_name;
    }

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
        // For a streaming owner_type adapter the call is dispatched on the owner
        // handle instance, not on the mock-backed `client` the factory constructs.
        let call_receiver = streaming_owner_handle.as_deref().unwrap_or("client");
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
                format!("{call_receiver}.{function_name}({args_str}){collect_suffix}")
            } else {
                format!("{call_receiver}.{function_name}({args_str})")
            };
            let _ = writeln!(out, "        assertFailsWith<Exception> {{");
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
            crate::e2e::codegen::error_path_assertions::emit(out, fixture, "        // ", lang);
            // Trailing `Unit` so the runBlocking { ... } lambda's final
            // expression is Unit (not the Exception returned by assertFailsWith).
            // The enclosing `fun ... = runBlocking { ... }` then infers Unit
            // as the test function's return type — JUnit 5 silently skips
            // any @Test method whose return type is not void/Unit.
            let _ = writeln!(out, "        Unit");
            let _ = writeln!(out, "    }}");
            return Ok(());
        }
        for line in &setup_lines {
            let _ = writeln!(out, "        {line}");
        }
        let _ = writeln!(
            out,
            "        val client = {class_name_for_call}.{factory}(apiKey = \"test-key\", baseUrl = {mock_url_expr})"
        );
        let _ = writeln!(
            out,
            "        val {result_var} = {call_receiver}.{function_name}({args_str})"
        );
        if !collect_snippet.is_empty() {
            let _ = writeln!(out, "        {collect_snippet}");
        }
        let assertions_start = out.len();
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
                json_scalar_fields,
                e2e_config.effective_fields_c_types(call_config),
                is_streaming,
                kotlin_android_style,
                not_error_may_assert_presence,
            );
        }
        crate::e2e::codegen::fail_on_unavailable_field_markers(
            &out[assertions_start..],
            lang,
            &fixture.id,
            &fixture.assertions,
        );
        crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&out[assertions_start..], lang, &fixture.id);
        refuse_inert_example(out, assertions_start, fixture, lang);
        let _ = writeln!(out, "        client.close()");
        let _ = writeln!(out, "    }}");
        return Ok(());
    }

    // Flat-function call style (no client_factory). For a streaming owner_type
    // adapter the call is an instance method on the owner handle rather than a
    // static facade call.
    let call_receiver = streaming_owner_handle.as_deref().unwrap_or(class_name_for_call);
    if expects_error {
        // Wrap setup + call in assertFailsWith so validation errors thrown
        // during engine creation are also caught (mirrors Java's assertThrows).
        let _ = writeln!(out, "        assertFailsWith<Exception> {{");
        for line in &setup_lines {
            let _ = writeln!(out, "            {line}");
        }
        let _ = writeln!(out, "            {call_receiver}.{function_name}({args_str})");
        let _ = writeln!(out, "        }}");
        crate::e2e::codegen::error_path_assertions::emit(out, fixture, "        // ", lang);
        // Trailing Unit — see comment in the client-factory branch above.
        let _ = writeln!(out, "        Unit");
        let _ = writeln!(out, "    }}");
        return Ok(());
    }

    for line in &setup_lines {
        let _ = writeln!(out, "        {line}");
    }

    let _ = writeln!(
        out,
        "        val {result_var} = {call_receiver}.{function_name}({args_str})"
    );

    if !collect_snippet.is_empty() {
        let _ = writeln!(out, "        {collect_snippet}");
    }

    let assertions_start = out.len();
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
            json_scalar_fields,
            &e2e_config.fields_c_types,
            is_streaming,
            kotlin_android_style,
            not_error_may_assert_presence,
        );
    }
    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&out[assertions_start..], lang, &fixture.id);
    crate::e2e::codegen::fail_on_unavailable_field_markers(
        &out[assertions_start..],
        lang,
        &fixture.id,
        &fixture.assertions,
    );
    refuse_inert_example(out, assertions_start, fixture, lang);

    let _ = writeln!(out, "    }}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::extras::{AdapterConfig, AdapterParam, AdapterPattern};
    use crate::e2e::config::{ArgMapping, CallConfig};

    fn handle_arg(name: &str) -> ArgMapping {
        ArgMapping {
            name: name.to_string(),
            field: format!("input.{name}"),
            arg_type: "handle".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    fn string_arg(name: &str) -> ArgMapping {
        ArgMapping {
            name: name.to_string(),
            field: format!("input.{name}"),
            arg_type: "string".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    /// Synthetic streaming `owner_type` adapter for `function_name`, matching
    /// the shape declared by `[[crates.adapters]]` in `alef.toml`.
    fn streaming_owner_adapter(function_name: &str, owner_type: &str) -> AdapterConfig {
        AdapterConfig {
            name: function_name.to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: format!("test_core::{function_name}"),
            params: Vec::new(),
            returns: None,
            error_type: None,
            owner_type: Some(owner_type.to_string()),
            item_type: Some("Item".to_string()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: None,
            skip_languages: Vec::new(),
        }
    }

    fn render(call: CallConfig, adapters: Vec<AdapterConfig>) -> String {
        let fixture = Fixture {
            id: "call_fixture".to_string(),
            description: "call fixture".to_string(),
            input: serde_json::json!({ "handle": {}, "url": "https://example.com" }),
            ..Fixture::default()
        };
        let e2e_config = E2eConfig {
            call,
            ..E2eConfig::default()
        };
        let config = ResolvedCrateConfig {
            adapters,
            ..ResolvedCrateConfig::default()
        };
        let mut out = String::new();
        render_test_method(
            &mut out,
            &fixture,
            "Facade",
            "",
            "",
            &[],
            None,
            false,
            &e2e_config,
            &std::collections::HashMap::new(),
            false,
            &config,
            &[],
            &[],
            &[],
        )
        .expect("render_test_method succeeds");
        out
    }

    /// A streaming call routed through a `[[crates.adapters]]` entry with
    /// `owner_type` set must be rendered as an instance method on the owner
    /// handle (`handle.streamItems(...)`), never as a static facade call that
    /// passes the handle as a positional argument. Mirrors the Java e2e
    /// backend's `streaming_owner_handle` behavior (java/test_method.rs).
    #[test]
    fn streaming_owner_type_adapter_uses_handle_as_instance_receiver() {
        let call = CallConfig {
            function: "stream_items".to_string(),
            result_var: "result".to_string(),
            args: vec![handle_arg("handle"), string_arg("url")],
            ..CallConfig::default()
        };
        let out = render(call, vec![streaming_owner_adapter("stream_items", "Engine")]);

        assert!(
            out.contains("val result = handle.streamItems(\"https://example.com\")"),
            "expected an instance call on the owner handle, got:\n{out}"
        );
        assert!(
            !out.contains("Facade.streamItems"),
            "must not emit a static facade call for a streaming owner_type adapter, got:\n{out}"
        );
        assert!(
            out.contains("val handle = Facade.createHandle(null)"),
            "the handle's construction line must still be emitted, got:\n{out}"
        );
        assert!(
            !out.contains("streamItems(handle, "),
            "the handle must not also appear as a positional argument, got:\n{out}"
        );
    }

    /// Regression guard: a call with no adapter entry (or an adapter that is
    /// not both `Streaming` and `owner_type`-bearing) must keep the plain
    /// static-facade call shape, with the handle passed positionally.
    #[test]
    fn non_owner_type_call_keeps_static_facade_call_with_handle_positional() {
        let call = CallConfig {
            function: "do_thing".to_string(),
            result_var: "result".to_string(),
            args: vec![handle_arg("handle"), string_arg("url")],
            ..CallConfig::default()
        };

        // No adapters at all.
        let out_no_adapter = render(call.clone(), Vec::new());
        assert!(
            out_no_adapter.contains("val result = Facade.doThing(handle, \"https://example.com\")"),
            "expected an unchanged static facade call, got:\n{out_no_adapter}"
        );

        // A non-streaming adapter for the same function must not trigger the
        // instance-receiver path either.
        let mut non_streaming = streaming_owner_adapter("do_thing", "Engine");
        non_streaming.pattern = AdapterPattern::SyncFunction;
        let out_sync = render(call.clone(), vec![non_streaming]);
        assert!(
            out_sync.contains("val result = Facade.doThing(handle, \"https://example.com\")"),
            "a non-streaming adapter must not switch to an instance receiver, got:\n{out_sync}"
        );

        // A streaming adapter with no owner_type must not trigger it either.
        let mut streaming_no_owner = streaming_owner_adapter("do_thing", "Engine");
        streaming_no_owner.owner_type = None;
        let out_no_owner = render(call, vec![streaming_no_owner]);
        assert!(
            out_no_owner.contains("val result = Facade.doThing(handle, \"https://example.com\")"),
            "a streaming adapter without owner_type must not switch to an instance receiver, got:\n{out_no_owner}"
        );
    }

    #[test]
    fn streaming_owner_call_builds_declared_request_type() {
        let call = CallConfig {
            function: "stream_items".to_string(),
            result_var: "result".to_string(),
            args: vec![handle_arg("handle"), string_arg("url")],
            ..CallConfig::default()
        };
        let mut adapter = streaming_owner_adapter("stream_items", "Engine");
        adapter.params.push(AdapterParam {
            name: "request".to_string(),
            ty: "sample::StreamRequest".to_string(),
            optional: false,
        });
        let generated = render(call, vec![adapter]);
        assert!(generated.contains("MAPPER.readValue("), "got:\n{generated}");
        assert!(generated.contains("StreamRequest::class.java"), "got:\n{generated}");
        assert!(generated.contains("handle.streamItems(request)"), "got:\n{generated}");
        assert!(!generated.contains("handle.streamItems(\"https://example.com\")"));

        let statements = generated
            .lines()
            .filter(|line| line.contains("val request =") || line.contains("val result ="))
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n    ");
        let source = format!(
            "data class StreamRequest(val url: String = \"\")\nclass Engine {{ fun streamItems(request: StreamRequest): List<String> = listOf(request.url) }}\nobject MAPPER {{ fun <T : Any> readValue(value: String, type: Class<T>): T = type.getDeclaredConstructor().newInstance() }}\nfun main() {{\n    val handle = Engine()\n    {statements}\n}}\n"
        );
        let directory = tempfile::tempdir().expect("temporary Kotlin compile directory");
        let source_path = directory.path().join("Assertions.kt");
        std::fs::write(&source_path, source).expect("write generated Kotlin source");
        // Pin the child's working directory. Other tests in this binary mutate the
        // process-global cwd via `set_current_dir` into tempdirs that are then dropped,
        // and a JVM launched with a deleted cwd dies in `SystemModuleFinders` before it
        // ever reads the source. ~keep
        let output = crate::core::tool_command("kotlinc")
            .arg(&source_path)
            .arg("-d")
            .arg(directory.path().join("assertions.jar"))
            .current_dir(directory.path())
            .output()
            .expect("run kotlinc");
        assert!(
            output.status.success(),
            "kotlinc failed ({})\n--- stdout ---\n{}\n--- stderr ---\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(
            !generated.contains("\\\"handle\\\""),
            "owner handle config leaked into request JSON: {generated}"
        );
    }

    fn test_backend_arg(name: &str, trait_name: &str) -> ArgMapping {
        ArgMapping {
            name: name.to_string(),
            field: format!("input.{name}"),
            arg_type: "test_backend".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: Some(trait_name.to_string()),
        }
    }

    /// Pin: when a `test_backend` arg's trait is registered in
    /// `config.trait_bridges`, kotlin_android must always splice a concrete stub
    /// instantiation into the call's argument list, never an unimplemented-emitter
    /// comment. This is the regression the round's `test_backend` bug class
    /// targets — if someone reintroduces an unchecked `parts.push(emission.arg_expr)`
    /// (or `kotlin_android::emit_test_backend` regresses to a stub that can no
    /// longer guarantee a real emission), this test fails.
    #[test]
    fn kotlin_android_test_backend_arg_renders_concrete_stub_instantiation() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{MethodDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};

        let trait_bridge = TraitBridgeConfig {
            trait_name: "Validator".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_validator".to_string()),
            ..Default::default()
        };
        let method = MethodDef {
            name: "validate".to_string(),
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..Default::default()
        };
        let type_def = TypeDef {
            name: "Validator".to_string(),
            methods: vec![method],
            ..Default::default()
        };
        let config = ResolvedCrateConfig {
            trait_bridges: vec![trait_bridge],
            ..ResolvedCrateConfig::default()
        };
        let call = CallConfig {
            function: "register_backend".to_string(),
            result_var: "result".to_string(),
            args: vec![test_backend_arg("backend", "Validator")],
            ..CallConfig::default()
        };
        let fixture = Fixture {
            id: "register_validator".to_string(),
            description: "register a validator backend".to_string(),
            input: serde_json::json!({ "backend": { "name": "test-validator" } }),
            ..Fixture::default()
        };
        let e2e_config = E2eConfig {
            call,
            ..E2eConfig::default()
        };

        let mut out = String::new();
        render_test_method(
            &mut out,
            &fixture,
            "Facade",
            "",
            "",
            &[],
            None,
            false,
            &e2e_config,
            &std::collections::HashMap::new(),
            true, // kotlin_android_style
            &config,
            &[type_def],
            &[],
            &[],
        )
        .expect("a registered, implemented trait bridge must render successfully");

        assert!(
            out.contains("TestStubRegisterValidator()"),
            "expected a concrete kotlin_android stub instantiation as the call argument, got:\n{out}"
        );
        assert!(
            !out.contains("/* test_backend unimplemented"),
            "must never splice the unimplemented sentinel into generated Kotlin, got:\n{out}"
        );
    }

    /// A `test_backend` arg whose trait has no matching `[[crates.trait_bridges]]`
    /// entry has no compilable value to fall back to (the `I{TraitName}` parameter
    /// is non-null) — generation must fail loudly instead of silently emitting
    /// `null` or a comment where the argument belongs.
    #[test]
    fn kotlin_android_test_backend_arg_with_unregistered_trait_fails_loudly() {
        let call = CallConfig {
            function: "register_backend".to_string(),
            result_var: "result".to_string(),
            args: vec![test_backend_arg("backend", "Validator")],
            ..CallConfig::default()
        };
        let fixture = Fixture {
            id: "register_validator".to_string(),
            description: "register a validator backend".to_string(),
            input: serde_json::json!({ "backend": { "name": "test-validator" } }),
            ..Fixture::default()
        };
        let e2e_config = E2eConfig {
            call,
            ..E2eConfig::default()
        };
        // No `trait_bridges` configured — "Validator" is unregistered.
        let config = ResolvedCrateConfig::default();

        let mut out = String::new();
        let error = render_test_method(
            &mut out,
            &fixture,
            "Facade",
            "",
            "",
            &[],
            None,
            false,
            &e2e_config,
            &std::collections::HashMap::new(),
            true,
            &config,
            &[],
            &[],
            &[],
        )
        .expect_err("an unregistered trait must fail generation loudly, not silently degrade");

        assert!(
            error.to_string().contains("Validator"),
            "error should name the unresolved trait, got: {error}"
        );
        assert!(
            !out.contains("null"),
            "must not have emitted a silent `null` placeholder before failing, got:\n{out}"
        );
    }

    fn json_object_arg(name: &str) -> ArgMapping {
        ArgMapping {
            name: name.to_string(),
            field: format!("input.{name}"),
            arg_type: "json_object".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    /// Regression test for alef #219: `render_test_method` used to precompute
    /// its own `val request = MAPPER.readValue(...)` binding (the now-removed
    /// `deser_lines` mechanism) *in addition to* the identical binding
    /// `build_args_and_setup` (args.rs) independently pushes onto
    /// `setup_lines`. Every generated Kotlin test with a `json_object` arg
    /// redeclared the same `val` twice, which Kotlin rejects as "Conflicting
    /// declarations" — 100% of the kotlin_android e2e suite failed to
    /// compile.
    ///
    /// This drives the interaction between both write sites at once: calling
    /// `render_test_method` (test_method.rs) exercises the removed
    /// `deser_lines` code path, and it internally calls
    /// `build_args_and_setup` (args.rs), which is the sole remaining
    /// emitter. A test that called `build_args_and_setup` directly (bypassing
    /// `render_test_method`) would never have observed the duplicate, since
    /// args.rs alone only ever emitted the binding once — only the pair
    /// together produced it twice. ~keep
    #[test]
    fn json_object_arg_emits_exactly_one_request_binding() {
        let call = CallConfig {
            function: "process".to_string(),
            result_var: "result".to_string(),
            args: vec![json_object_arg("request")],
            ..CallConfig::default()
        };
        let fixture = Fixture {
            id: "process_request".to_string(),
            description: "process a request".to_string(),
            input: serde_json::json!({ "request": { "kind": "text", "value": "hello" } }),
            ..Fixture::default()
        };
        let e2e_config = E2eConfig {
            call,
            ..E2eConfig::default()
        };
        let config = ResolvedCrateConfig::default();

        let mut out = String::new();
        render_test_method(
            &mut out,
            &fixture,
            "Facade",
            "",
            "",
            &[],
            Some("Request"),
            false,
            &e2e_config,
            &std::collections::HashMap::new(),
            false,
            &config,
            &[],
            &[],
            &[],
        )
        .expect("render_test_method succeeds");

        // Property 1: the fixture must actually produce a request binding at all —
        // otherwise the `== 1` count below would pass vacuously.
        assert!(
            out.contains("val request = MAPPER.readValue("),
            "expected a request deserialization binding, got:\n{out}"
        );
        let binding_count = out.matches("val request =").count();
        assert_eq!(
            binding_count, 1,
            "expected exactly one `val request =` binding, got {binding_count}:\n{out}"
        );
    }

    /// Same regression as `json_object_arg_emits_exactly_one_request_binding`, but
    /// for the `expects_error` path: both write sites used to place their binding
    /// INSIDE the `assertFailsWith` block (so Jackson validation errors on the
    /// request literal are caught by the test), and both did so independently —
    /// the duplicate reproduced there too, on a code path the happy-path test
    /// above never touches (`expects_error` selects an entirely different emission
    /// branch in test_method.rs). ~keep
    #[test]
    fn json_object_arg_emits_exactly_one_request_binding_in_error_test() {
        let call = CallConfig {
            function: "process".to_string(),
            result_var: "result".to_string(),
            args: vec![json_object_arg("request")],
            ..CallConfig::default()
        };
        let fixture = Fixture {
            id: "process_request_error".to_string(),
            description: "reject an invalid request".to_string(),
            input: serde_json::json!({ "request": { "kind": "invalid", "value": "hello" } }),
            assertions: vec![crate::e2e::fixture::Assertion {
                assertion_type: "error".to_string(),
                ..Default::default()
            }],
            ..Fixture::default()
        };
        let e2e_config = E2eConfig {
            call,
            ..E2eConfig::default()
        };
        let config = ResolvedCrateConfig::default();

        let mut out = String::new();
        render_test_method(
            &mut out,
            &fixture,
            "Facade",
            "",
            "",
            &[],
            Some("Request"),
            false,
            &e2e_config,
            &std::collections::HashMap::new(),
            false,
            &config,
            &[],
            &[],
            &[],
        )
        .expect("render_test_method succeeds");

        assert!(
            out.contains("val request = MAPPER.readValue("),
            "expected a request deserialization binding inside assertFailsWith, got:\n{out}"
        );
        let binding_count = out.matches("val request =").count();
        assert_eq!(
            binding_count, 1,
            "expected exactly one `val request =` binding, got {binding_count}:\n{out}"
        );
    }

    fn render_error_method(extra: Vec<crate::e2e::fixture::Assertion>) -> String {
        let mut assertions = vec![crate::e2e::fixture::Assertion {
            assertion_type: "error".to_string(),
            ..Default::default()
        }];
        assertions.extend(extra);
        let fixture = Fixture {
            id: "rate_limited".to_string(),
            description: "reject the request".to_string(),
            input: serde_json::json!({}),
            assertions,
            ..Fixture::default()
        };
        let e2e_config = E2eConfig {
            call: CallConfig {
                function: "process".to_string(),
                result_var: "result".to_string(),
                ..CallConfig::default()
            },
            ..E2eConfig::default()
        };
        let config = ResolvedCrateConfig::default();
        let mut out = String::new();
        let _ = crate::e2e::codegen::take_skip_records();
        render_test_method(
            &mut out,
            &fixture,
            "Facade",
            "",
            "",
            &[],
            None,
            false,
            &e2e_config,
            &std::collections::HashMap::new(),
            false,
            &config,
            &[],
            &[],
            &[],
        )
        .expect("render_test_method succeeds");
        out
    }

    /// Kotlin's error path emits `assertFailsWith<Exception> { .. }` and returns, so every other
    /// assertion on the fixture used to leave no trace in the generated test at all.
    #[test]
    fn kotlin_equals_on_an_error_field_is_named_instead_of_dropped() {
        let out = render_error_method(vec![crate::e2e::fixture::Assertion {
            assertion_type: "equals".to_string(),
            field: Some("error.status_code".to_string()),
            ..Default::default()
        }]);

        // Positive first: the error block really rendered.
        assert!(
            out.contains("assertFailsWith<Exception> {"),
            "the error block must render: {out}"
        );
        assert!(
            out.contains(
                "// skipped: assertion type 'equals' has no accessor for error field error.status_code in this \
                 backend"
            ),
            "{out}"
        );

        let records = crate::e2e::codegen::take_skip_records();
        assert_eq!(records.len(), 1, "got: {records:?}");
        assert_eq!(records[0].language, "kotlin");
        assert_eq!(records[0].field, "equals");
    }

    /// Negative control: a lone `error` assertion must leave the generated method marker-free.
    #[test]
    fn kotlin_a_lone_error_assertion_renders_no_marker() {
        let out = render_error_method(Vec::new());

        assert!(
            out.contains("assertFailsWith<Exception> {"),
            "the error block must render: {out}"
        );
        assert!(!out.contains("has no accessor for error field"), "{out}");
    }
}

#[cfg(test)]
mod inert_example_refusal_tests;

#[cfg(test)]
mod junit_unit_return_tests;
