//! Elixir ordinary function-call e2e test rendering.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_ident;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use std::collections::HashMap;
use std::fmt::Write as _;

use super::args::build_args_and_setup;
use super::assertions::render_assertion;
use super::visitor::build_elixir_visitor;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_case(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    _default_module_path: &str,
    _default_function_name: &str,
    _default_result_var: &str,
    _args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
    options_default_fn: Option<&str>,
    _enum_fields: &HashMap<String, String>,
    handle_struct_type: Option<&str>,
    _handle_atom_list_fields: &std::collections::HashSet<String>,
    adapters: &[crate::core::config::extras::AdapterConfig],
    enums: &[crate::core::ir::EnumDef],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) {
    let test_name = sanitize_ident(&fixture.id);
    let test_label = fixture.id.replace('"', "\\\"");

    // Helper function to extract module-level definitions from a setup_block that may
    // contain a trait-bridge marker. Trait-bridge setup blocks are formatted as:
    //   <module definitions ending with "end\n">
    //   \n__TRAIT_BRIDGE_MODULE_DEFS_END__\n
    //   <test-function-level setup>
    // We split on the marker and emit module defs before the test, then use only the setup part.
    fn extract_trait_bridge_parts(setup_block: &str) -> (String, String) {
        if let Some(pos) = setup_block.find("__TRAIT_BRIDGE_MODULE_DEFS_END__") {
            // Find the start and end of the marker line
            let marker_start = setup_block[..pos].rfind('\n').unwrap_or(0);
            let marker_end = if let Some(nl) = setup_block[pos + 32..].find('\n') {
                pos + 32 + nl + 1
            } else {
                setup_block.len()
            };
            let module_defs = setup_block[..marker_start].trim_end().to_string();
            let test_setup = setup_block[marker_end..].trim_start().to_string();
            (module_defs, test_setup)
        } else {
            // No marker: entire block is test-level setup (legacy or non-trait-bridge code)
            (String::new(), setup_block.to_string())
        }
    }

    // Non-HTTP non-mock_response fixtures (e.g. AsyncAPI, WebSocket, OpenRPC
    // protocol-only fixtures) cannot be tested via the configured `[e2e.call]`
    // function when the binding does not expose it. Emit a documented `@tag :skip`
    // test so the suite stays compilable. HTTP fixtures dispatch via render_http_test_case
    // and never reach here.
    if fixture.mock_response.is_none() && !fixture_has_elixir_callable(fixture, e2e_config) {
        let _ = writeln!(out, "  describe \"{test_name}\" do");
        let _ = writeln!(out, "    @tag :skip");
        let _ = writeln!(out, "    test \"{test_label}\" do");
        let _ = writeln!(
            out,
            "      # non-HTTP fixture: Elixir binding does not expose a callable for the configured `[e2e.call]` function"
        );
        let _ = writeln!(out, "      :ok");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
        return;
    }

    // Resolve per-fixture call config (falls back to default if fixture.call is None).
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
        &std::collections::HashSet::new(),
    );
    let field_resolver = &call_field_resolver;
    let lang = "elixir";
    let call_overrides = call_config.overrides.get(lang);

    // Check if the function is excluded from the Elixir binding (e.g., batch functions
    // that require unsafe NIF tuple marshalling). Emit a skipped test with rationale.
    let base_fn = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    if base_fn.starts_with("batch_extract_") {
        let _ = writeln!(
            out,
            "  describe \"{test_name}\" do",
            test_name = sanitize_ident(&fixture.id)
        );
        let _ = writeln!(out, "    @tag :skip");
        let _ = writeln!(
            out,
            "    test \"{test_label}\" do",
            test_label = fixture.id.replace('"', "\\\"")
        );
        let _ = writeln!(
            out,
            "      # batch functions excluded from Elixir binding: unsafe NIF tuple marshalling"
        );
        let _ = writeln!(out, "      :ok");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
        return;
    }

    // Compute module_path and function_name from the resolved call config.
    // call_config is resolved via resolve_call_for_fixture which applies select_when auto-routing,
    // so we always use it - whether or not fixture.call was explicitly set.
    // Apply Elixir-specific PascalCase conversion.
    let raw_module = call_overrides
        .and_then(|o| o.module.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.module.clone());
    let module_path = if raw_module.contains('.') || raw_module.chars().next().is_some_and(|c| c.is_uppercase()) {
        raw_module
    } else {
        super::values::elixir_module_name(&raw_module)
    };
    let function_name = if call_config.r#async && !base_fn.ends_with("_async") && !base_fn.ends_with("_stream") {
        format!("{base_fn}_async")
    } else {
        base_fn
    };
    let result_var = call_config.result_var.clone();

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    // Validation-category fixtures expect engine creation itself to fail (bad config).
    // Other expects_error fixtures (e.g. error_*) construct a valid engine and expect the
    // *operation under test* to fail. We need different shapes for these two cases.
    let validation_creation_failure = expects_error && fixture.resolved_category() == "validation";

    // Use args and options from the resolved call_config (which may have been auto-routed via select_when),
    // falling back to the fixture-level defaults if not available.
    let co = call_config.overrides.get(lang);
    let empty_enum_fields_local: HashMap<String, String> = HashMap::new();
    let empty_atom_fields_local: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Use the call config's args, not the fallback global args.
    // This ensures that functions like list_document_extractors with args=[] stay empty,
    // instead of falling back to the global [crates.e2e.call] args which are meant for extract_file.
    let resolved_args = fixture.resolved_args(call_config);
    let resolved_options_type = co
        .and_then(|o| o.options_type.clone())
        .or_else(|| options_type.map(|s| s.to_string()));
    let resolved_options_default_fn = co
        .and_then(|o| o.options_via.clone())
        .or_else(|| options_default_fn.map(|s| s.to_string()));
    let resolved_enum_fields_ref = co.map(|o| &o.enum_fields).unwrap_or(&empty_enum_fields_local);
    let resolved_handle_struct_type = co
        .and_then(|o| o.handle_struct_type.clone())
        .or_else(|| handle_struct_type.map(|s| s.to_string()));
    let resolved_handle_atom_list_fields_ref = co
        .map(|o| &o.handle_atom_list_fields)
        .unwrap_or(&empty_atom_fields_local);

    let test_documents_path = e2e_config.test_documents_relative_from(0);
    let adapter_request_type: Option<String> = adapters
        .iter()
        .find(|a| a.name == call_config.function.as_str())
        .and_then(|a| a.request_type.as_deref())
        .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());
    let (mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        resolved_args,
        &module_path,
        resolved_options_type.as_deref(),
        resolved_options_default_fn.as_deref(),
        resolved_enum_fields_ref,
        fixture,
        resolved_handle_struct_type.as_deref(),
        resolved_handle_atom_list_fields_ref,
        &test_documents_path,
        adapter_request_type.as_deref(),
        enums,
        config,
        type_defs,
    );

    // Build visitor if present - it will be injected into the options map.
    let visitor_var = fixture
        .visitor
        .as_ref()
        .map(|visitor_spec| build_elixir_visitor(&mut setup_lines, visitor_spec));

    // If we have a visitor and the args contain a nil for options, replace it with a map
    // containing the visitor. The fixture.visitor is already set above.
    let final_args = if let Some(ref visitor_var) = visitor_var {
        // Parse args_str to handle injection properly.
        // Since we're dealing with a 2-arg function (html, options), and options might be nil,
        // we need to inject visitor into the options.
        let parts: Vec<&str> = args_str.split(", ").collect();
        if parts.len() == 2 && parts[1] == "nil" {
            // Replace nil with %{visitor: visitor}
            format!("{}, %{{visitor: {}}}", parts[0], visitor_var)
        } else if parts.len() == 2 {
            // Options is a variable (e.g., "options") - add visitor to it
            setup_lines.push(format!(
                "{} = Map.put({}, :visitor, {})",
                parts[1], parts[1], visitor_var
            ));
            args_str
        } else if parts.len() == 1 {
            // Only HTML provided - create options map with just visitor
            format!("{}, %{{visitor: {}}}", parts[0], visitor_var)
        } else {
            args_str
        }
    } else {
        args_str
    };

    // Client factory: when configured, create a client and pass it as the first argument.
    let client_factory = call_overrides.and_then(|o| o.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get("elixir")
            .and_then(|o| o.client_factory.as_deref())
    });

    // Append per-call extra_args (e.g. trailing `nil` for `list_files(client, query)`)
    // so Elixir matches the binding's positional arity. Mirrors the same override the
    // Ruby/Go/Node codegens already honor.
    let extra_args: Vec<String> = call_overrides.map(|o| o.extra_args.clone()).unwrap_or_default();
    let final_args_with_extras = if extra_args.is_empty() {
        final_args
    } else if final_args.is_empty() {
        extra_args.join(", ")
    } else {
        format!("{final_args}, {}", extra_args.join(", "))
    };

    // Prefix the client variable to the args when client_factory is set.
    let effective_args = if client_factory.is_some() {
        if final_args_with_extras.is_empty() {
            "client".to_string()
        } else {
            format!("client, {final_args_with_extras}")
        }
    } else {
        final_args_with_extras
    };

    // Real-API smoke fixtures (no mock_response, no http) must be env-gated on the
    // configured `env.api_key_var` so absent keys yield a deterministic skip rather
    // than a spurious "no mock route" failure. Mirrors the Python conftest skip.
    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var_opt = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    let needs_api_key_skip = !has_mock && api_key_var_opt.is_some();
    // When the fixture has both a mock and an api_key_var, generate env-fallback code:
    // use the real API when the key is set, otherwise fall back to the mock server.
    let needs_env_fallback = has_mock && api_key_var_opt.is_some();

    // Extract trait-bridge module definitions from setup_lines and keep only the test-level parts.
    // Trait-bridge setup blocks are formatted with a marker: module defs, then marker, then test setup.
    // Module defs are emitted at file level by render_test_file, so we only keep the test-level setup here.
    let mut cleaned_setup_lines = Vec::new();
    for line in setup_lines.iter() {
        if line.contains("__TRAIT_BRIDGE_MODULE_DEFS_END__") {
            // Split this line on the marker and discard the module-level part
            let (_module_part, test_part) = extract_trait_bridge_parts(line);
            // Emit test-level part indented in the test function
            for test_line in test_part.lines() {
                if !test_line.is_empty() {
                    cleaned_setup_lines.push(test_line.to_string());
                }
            }
        } else {
            cleaned_setup_lines.push(line.clone());
        }
    }

    let _ = writeln!(out, "  describe \"{test_name}\" do");
    let _ = writeln!(out, "    test \"{test_label}\" do");

    if needs_api_key_skip {
        let api_key_var = api_key_var_opt.unwrap_or("");
        let _ = writeln!(out, "      if System.get_env(\"{api_key_var}\") in [nil, \"\"] do");
        let _ = writeln!(out, "        # {api_key_var} not set — skipping live smoke test");
        let _ = writeln!(out, "        :ok");
        let _ = writeln!(out, "      else");
    }

    // Validation-category fixtures: engine/handle creation itself is expected to fail.
    // Transform the first `{:ok, _} = ...` setup line into `assert {:error, _} = ...`
    // and stop emission there, since the rest of the test body would be unreachable.
    if validation_creation_failure {
        let mut emitted_error_assertion = false;
        for line in &cleaned_setup_lines {
            if !emitted_error_assertion && line.starts_with("{:ok,") {
                if let Some(rhs) = line.split_once('=').map(|x| x.1) {
                    let rhs = rhs.trim();
                    let _ = writeln!(out, "      assert {{:error, _}} = {rhs}");
                    emitted_error_assertion = true;
                } else {
                    let _ = writeln!(out, "      {line}");
                }
            } else {
                let _ = writeln!(out, "      {line}");
            }
        }
        if !emitted_error_assertion {
            let call_invocation = if effective_args.is_empty() {
                format!("{module_path}.{function_name}()")
            } else {
                format!("{module_path}.{function_name}({effective_args})")
            };
            let _ = writeln!(out, "      assert {{:error, _}} = {call_invocation}");
        }
        if needs_api_key_skip {
            let _ = writeln!(out, "      end");
        }
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
        return;
    }

    // Non-validation expects_error fixtures (error_*, etc.): engine creation succeeds.
    // Emit setup as-is and assert that the *operation under test* fails. The
    // call body still references `client` (e.g. `defaultclient_chat_async(client, ...)`),
    // so we must emit the same `{:ok, client} = create_client(...)` line that the
    // non-error path below uses - without it the generated test fails to compile
    // with `undefined variable "client"`.
    if expects_error {
        for line in &cleaned_setup_lines {
            let _ = writeln!(out, "      {line}");
        }
        if let Some(factory) = client_factory {
            let fixture_id = &fixture.id;
            let base_url_expr = if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                format!(
                    "(System.get_env(\"{env_key}\") || (System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\")"
                )
            } else {
                format!("(System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\"")
            };
            let _ = writeln!(
                out,
                "      {{:ok, client}} = {module_path}.{factory}(\"test-key\", base_url: {base_url_expr})"
            );
        }
        let call_invocation = if effective_args.is_empty() {
            format!("{module_path}.{function_name}()")
        } else {
            format!("{module_path}.{function_name}({effective_args})")
        };
        let _ = writeln!(out, "      assert {{:error, _}} = {call_invocation}");
        if needs_api_key_skip {
            let _ = writeln!(out, "      end");
        }
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
        return;
    }

    for line in &cleaned_setup_lines {
        let _ = writeln!(out, "      {line}");
    }

    // Emit client creation when client_factory is configured.
    if let Some(factory) = client_factory {
        let fixture_id = &fixture.id;
        if needs_env_fallback {
            // Fixture has both a mock and an api_key_var: use the real API when the key is
            // set, otherwise fall back to the mock server.
            let api_key_var = api_key_var_opt.unwrap_or("");
            let mock_url_expr = if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                format!(
                    "System.get_env(\"{env_key}\") || (System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\""
                )
            } else {
                format!("(System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\"")
            };
            let _ = writeln!(out, "      api_key_val = System.get_env(\"{api_key_var}\")");
            let _ = writeln!(
                out,
                "      {{api_key_val, client_opts}} = if api_key_val && api_key_val != \"\" do"
            );
            let _ = writeln!(
                out,
                "        IO.puts(\"{fixture_id}: using real API ({api_key_var} is set)\")"
            );
            let _ = writeln!(out, "        {{api_key_val, []}}");
            let _ = writeln!(out, "      else");
            let _ = writeln!(
                out,
                "        IO.puts(\"{fixture_id}: using mock server ({api_key_var} not set)\")"
            );
            let _ = writeln!(out, "        {{\"test-key\", [base_url: {mock_url_expr}]}}");
            let _ = writeln!(out, "      end");
            let _ = writeln!(
                out,
                "      {{:ok, client}} = {module_path}.{factory}(api_key_val, client_opts)"
            );
        } else {
            let base_url_expr = if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                format!(
                    "(System.get_env(\"{env_key}\") || (System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\")"
                )
            } else {
                format!("(System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\"")
            };
            let _ = writeln!(
                out,
                "      {{:ok, client}} = {module_path}.{factory}(\"test-key\", base_url: {base_url_expr})"
            );
        }
    }

    // Use returns_result from the Elixir override if present, otherwise from base config
    let returns_result = call_overrides
        .and_then(|o| o.returns_result)
        .unwrap_or(call_config.returns_result || client_factory.is_some());

    // Some calls (e.g. speech, file_content) return raw bytes rather than a struct.
    // When the call is marked `result_is_simple`, treat the bound `result` variable as
    // the value itself so assertions on a logical "audio"/"content" field map to the
    // bare binary instead of a struct accessor that doesn't exist.
    let result_is_simple = call_config.result_is_simple || call_overrides.is_some_and(|o| o.result_is_simple);

    // Streaming detection (call-level `streaming` opt-out is honored).
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());
    // For streaming fixtures the stream is bound to `result_var` first, then drained into `chunks`.
    let chunks_var = "chunks";

    // If the result variable is never referenced in assertions or streaming operations,
    // prefix it with _ to avoid "unused variable" warnings in mix compile --warnings-as-errors.
    let actual_result_var = if fixture.assertions.is_empty() && !is_streaming {
        format!("_{result_var}")
    } else {
        result_var.to_string()
    };

    // Render function call: omit args entirely if effective_args is empty (no-arg functions).
    // This prevents emitting `func(nil)` which causes FunctionClauseError on nil-free function signatures.
    let call_invocation = if effective_args.is_empty() {
        format!("{module_path}.{function_name}()")
    } else {
        format!("{module_path}.{function_name}({effective_args})")
    };

    if returns_result {
        let _ = writeln!(out, "      {{:ok, {actual_result_var}}} = {call_invocation}");
    } else {
        // Non-Result function returns value directly (e.g., bool, String).
        let _ = writeln!(out, "      {actual_result_var} = {call_invocation}");
    }

    // For streaming fixtures, drain the stream into a list before asserting.
    if is_streaming {
        if let Some(collect) = crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet(
            "elixir",
            &result_var,
            chunks_var,
        ) {
            let _ = writeln!(out, "      {collect}");
        }
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            if is_streaming { chunks_var } else { &result_var },
            field_resolver,
            &module_path,
            e2e_config.effective_fields_enum(call_config),
            resolved_enum_fields_ref,
            result_is_simple,
            is_streaming,
        );
    }

    if needs_api_key_skip {
        let _ = writeln!(out, "      end");
    }
    let _ = writeln!(out, "    end");
    let _ = writeln!(out, "  end");
}

fn fixture_has_elixir_callable(fixture: &Fixture, e2e_config: &E2eConfig) -> bool {
    // HTTP fixtures are handled separately - not our concern here.
    if fixture.is_http_test() {
        return false;
    }
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let elixir_override = call_config
        .overrides
        .get("elixir")
        .or_else(|| e2e_config.call.overrides.get("elixir"));
    // When a client_factory is configured the fixture is callable via the client pattern.
    if elixir_override.and_then(|o| o.client_factory.as_deref()).is_some() {
        return true;
    }
    // Elixir bindings expose functions via module-level callables.
    // Like Python and Node, Elixir can call the base function directly without requiring
    // a language-specific override. The function can come from either the override or
    // the default [e2e.call] configuration.
    let function_from_override = elixir_override.and_then(|o| o.function.as_deref());

    // If there's an override function, use it. Otherwise, Elixir can use the base function.
    function_from_override.is_some() || !call_config.function.is_empty()
}
