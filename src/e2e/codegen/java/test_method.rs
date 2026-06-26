use crate::core::config::ResolvedCrateConfig;
use crate::core::config::extras::AdapterConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::escape_java;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use heck::{ToLowerCamelCase, ToUpperCamelCase};

use super::args::{JavaArgsContext, build_args_and_setup};
use super::assertions::render_assertion;
use super::http::render_http_test_method;
use super::values::{java_builder_expression, json_to_java};
use super::visitor::{apply_java_visitor_arg, build_java_visitor, java_visitor_binding};

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
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone());
    let field_resolver = &call_field_resolver;
    let effective_enum_fields = e2e_config.effective_fields_enum(call_config);
    let enum_fields = effective_enum_fields;
    let lang = "java";
    let call_overrides = call_config.overrides.get(lang);
    let effective_function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.to_lower_camel_case());
    let effective_result_var = &call_config.result_var;
    let function_name = effective_function_name.as_str();
    let result_var = effective_result_var.as_str();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs);
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
                        let escaped = escape_java(&json_str);
                        let var_name = &arg.name;
                        if crate::e2e::codegen::value_contains_mock_url_placeholder(&normalized) {
                            let env_key = crate::e2e::codegen::mock_url_env_key(&fixture.id);
                            builder_expressions.push_str(&format!(
                                "        String {var_name}MockBaseUrl = System.getProperty(\"mockServer.{fixture_id}\", System.getenv().getOrDefault(\"{env_key}\", System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\")) + \"/fixtures/{fixture_id}\"));\n",
                                fixture_id = fixture.id,
                            ));
                            builder_expressions.push_str(&format!(
                                "        String {var_name}Json = \"{escaped}\".replace(\"{}\", {var_name}MockBaseUrl);\n",
                                crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                            ));
                            builder_expressions.push_str(&format!(
                                "        var {var_name} = JsonUtil.fromJson({var_name}Json, {opts_type}.class);\n",
                            ));
                        } else {
                            builder_expressions.push_str(&format!(
                                "        var {var_name} = JsonUtil.fromJson(\"{escaped}\", {opts_type}.class);\n",
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
                        );
                        let var_name = &arg.name;
                        builder_expressions.push_str(&format!("        var {} = {};\n", var_name, builder_expr));
                    }
                }
            }
        }
    }

    let adapter = adapters.iter().find(|a| a.name == call_config.function.as_str());
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
    if needs_source_var {
        if let Some(source_arg) = args.iter().find(|a| a.field == "source_code") {
            let field = source_arg.field.strip_prefix("input.").unwrap_or(&source_arg.field);
            if let Some(val) = fixture.input.get(field) {
                let java_val = json_to_java(val);
                assertions_body.push_str(&format!("        var source = {}.getBytes();\n", java_val));
            }
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
        crate::e2e::codegen::recipe::streaming_item_type(call_config, adapters, &[call_config.function.as_str()]);

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
        );
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

    let rendered = crate::e2e::template_env::render(
        "java/test_method.jinja",
        minijinja::context! {
            method_name => method_name,
            description => description,
            builder_expressions => builder_expressions,
            setup_lines => combined_setup,
            throws_clause => throws_clause,
            expects_error => expects_error,
            call_expr => call_expr,
            result_var => result_var,
            returns_void => call_config.returns_void,
            collect_snippet => collect_snippet,
            assertions_body => assertions_body,
            teardown_block => teardown_block,
        },
    );
    out.push_str(&rendered);
}
