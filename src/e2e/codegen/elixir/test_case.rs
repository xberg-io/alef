//! Elixir ordinary function-call e2e test rendering.
//!
//! ~keep This file is already over the repo's 1,000-line file-modularization cap. The
//! `not_error_may_assert_presence` unification (routing through
//! `not_error_presence::may_assert_presence`) touched `render_test_case` and
//! `apply_vacuous_assertion_fallback` directly and could not be split out — both are
//! tightly coupled to this function's local control flow (the underscore-prefixing decision
//! for `actual_result_var` and the vacuous-assertion fallback both need the same
//! already-in-scope `call_config`/`fixture` locals) — so this file grew by a small,
//! bounded amount of production logic. The accompanying regression test was moved to a new
//! sibling file (`not_error_bare_option_underscoring_tests.rs`) specifically to avoid adding
//! to that growth.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::call_ir::{CallIr, resolve_declared_result_type};
use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, skip_line};
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{escape_elixir, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use std::collections::HashMap;
use std::fmt::Write as _;

use super::args::build_args_and_setup;
use super::assertions::render_assertion;
use super::visitor::build_elixir_visitor;

/// Emit an `assert {:error, _}` check for a call expression under `indent`.
///
/// ~keep When a declared error value exists, `inspect/1` is used rather than
/// `to_string/1` because the reason may not implement `String.Chars` (an atom
/// or struct). `inspect/1` renders a String reason as its quoted message text
/// and an atom/struct reason as its type/variant name, so a single substring
/// check enforces the same message-OR-type disjunction the other language
/// backends apply explicitly (see `declared_error_value` in codegen/mod.rs) —
/// for the fixtures `declared_error_variant::classify` recognises as message-style. A value
/// naming a real `ErrorVariant` renders the registered skip instead: the NIF boundary
/// collapses every reason to a String via `.map_err(|e| e.to_string())`, so `inspect/1` never
/// actually sees an atom or struct to report a variant's identity through.
fn emit_error_assertion(
    out: &mut String,
    indent: &str,
    call_expr: &str,
    fixture: &Fixture,
    errors: &[crate::core::ir::ErrorDef],
) {
    match classify("elixir", fixture, errors) {
        DeclaredErrorAssertion::Undeclared => {
            let _ = writeln!(out, "{indent}assert {{:error, _}} = {call_expr}");
        }
        DeclaredErrorAssertion::Assert(value) => {
            let escaped = escape_elixir(value);
            let _ = writeln!(out, "{indent}assert {{:error, __reason}} = {call_expr}");
            let _ = writeln!(out, "{indent}assert String.contains?(inspect(__reason), \"{escaped}\")");
        }
        DeclaredErrorAssertion::Unsubstantiable(variant) => {
            let _ = writeln!(out, "{indent}assert {{:error, __reason}} = {call_expr}");
            let _ = writeln!(out, "{indent}{}", skip_line("", "#", variant, &fixture.id, "elixir"));
        }
    }
}

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
    errors: &[crate::core::ir::ErrorDef],
    functions: &[crate::core::ir::FunctionDef],
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
    let lang = "elixir";
    // Build per-call field resolver using the effective field sets for this call.
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    // Anchor the IR-derived enum classification (`with_ir_enum_map`) at the call's declared
    // Rust return type so a leaf field name that means different things on different types
    // resolves per owner, mirroring the rust/csharp/gleam/swift/dart e2e generators. This is
    // purely additive: `is_enum` still consults `with_enum_fields` (the hand-maintained
    // `fields_enum` config) FIRST, so an explicit config entry always wins. ~keep
    let call_root_type = resolve_declared_result_type(call_config, lang, CallIr { functions, type_defs });
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
    .with_ir_result_fields(FieldResolver::ir_result_field_facts(type_defs, lang), call_root_type)
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields);
    let field_resolver = &call_field_resolver;
    let call_overrides = call_config.overrides.get(lang);
    // WHETHER `not_error` may assert presence is decided once, centrally — see
    // `not_error_presence::may_assert_presence`'s doc for why a sibling assertion or an
    // `Option<T>` result both make an unconditional presence check unsafe. Resolved this early
    // (rather than just above the `render_assertion` loop) because `actual_result_var`'s own
    // underscore-prefixing decision below also depends on it: when this is `false` for a
    // fixture whose only assertion is `not_error`, `render_assertion` renders nothing, and the
    // `{:ok, result} = call(...)` binding would otherwise go unreferenced. ~keep
    let not_error_result_is_option = call_config.result_is_option || call_overrides.is_some_and(|o| o.result_is_option);
    let not_error_may_assert_presence =
        crate::e2e::codegen::not_error_presence::may_assert_presence(fixture, not_error_result_is_option);

    // Batch-fn skip removed: the rustler backend now supports `batch_extract_*` via JSON
    // parameter marshalling (Vec<Named> → Option<String> JSON, deserialized in the NIF
    // preamble) and the auto-encoded `Vec<ExtractionResult>` return.  Tests are emitted
    // normally; if a downstream still has a real NIF-side gap the call will fail at
    // runtime with a real error rather than being silently skipped.
    let base_fn = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());

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
    let result_var = call_config.effective_result_var().to_string();

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
    let force_keyword_args = call_overrides.is_some_and(|o| o.keyword_args)
        || e2e_config.call.overrides.get(lang).is_some_and(|o| o.keyword_args);
    let (mut setup_lines, args_str, teardown_block) = build_args_and_setup(
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
        force_keyword_args,
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
    // Streaming entry points (e.g. `chat_stream`) drive the FFI iterator handle and are not
    // async-callable in the OpenAI sense — the binding exposes them under their base name, so the
    // e2e must not append `_async`. Mirrors the guard at src/e2e/codegen/elixir.rs.
    let function_name = if call_config.r#async
        && client_factory.is_some()
        && !base_fn.ends_with("_async")
        && !base_fn.ends_with("_stream")
    {
        format!("{base_fn}_async")
    } else {
        base_fn
    };

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

    // Register the trait-bridge teardown immediately after the GenServer starts (via
    // `on_exit/1`), in every code path (validation-failure, expects-error, normal) below.
    // `on_exit` runs even if the test body raises or an assertion fails partway through,
    // unlike a plain trailing statement placed after the call+assertions — necessary here
    // because ExUnit shares one BEAM VM (and one Rust-side plugin registry) across the
    // whole suite. See `emit_test_backend`'s doc comment for the full rationale.
    if !teardown_block.is_empty() {
        for line in teardown_block.lines() {
            if !line.is_empty() {
                cleaned_setup_lines.push(line.to_string());
            }
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

    // Validation-category fixtures may fail either at engine/handle construction (bad
    // config) or per-request on the operation under test (e.g. an SSRF policy checked
    // against the target URL, not the config). We cannot know which applies from the
    // fixture alone, so both must be exercised: attempt creation, and only if it
    // succeeds go on to call the operation and assert the error there. ~keep A version
    // that always stopped after asserting `{:error, _}` on creation made every
    // per-request-error validation fixture vacuously pass (the operation was never
    // called) — see the elixir-only `skip` on a consumer's `validation_ssrf_*` fixtures,
    // which documents this exact bug and is meant to be lifted once this is fixed.
    if validation_creation_failure {
        let handle_arg_name = resolved_args.iter().find(|a| a.arg_type == "handle").map(|a| &a.name);
        let create_line_idx = handle_arg_name.and_then(|name| {
            let prefix = format!("{{:ok, {name}}}");
            cleaned_setup_lines.iter().position(|line| line.starts_with(&prefix))
        });

        if let Some(idx) = create_line_idx {
            for line in &cleaned_setup_lines[..idx] {
                let _ = writeln!(out, "      {line}");
            }
            let line = &cleaned_setup_lines[idx];
            let rhs = line.split_once('=').map(|(_, r)| r.trim()).unwrap_or(line.as_str());
            let bound_var = handle_arg_name.expect("create_line_idx is Some only when handle_arg_name is Some");

            let _ = writeln!(out, "      case {rhs} do");
            let _ = writeln!(out, "        {{:error, __reason}} ->");
            match classify("elixir", fixture, errors) {
                DeclaredErrorAssertion::Undeclared => {
                    let _ = writeln!(out, "          :ok");
                }
                DeclaredErrorAssertion::Assert(value) => {
                    let escaped = escape_elixir(value);
                    let _ = writeln!(
                        out,
                        "          assert String.contains?(inspect(__reason), \"{escaped}\")"
                    );
                }
                DeclaredErrorAssertion::Unsubstantiable(variant) => {
                    let _ = writeln!(out, "          {}", skip_line("", "#", variant, &fixture.id, "elixir"));
                }
            }
            let _ = writeln!(out, "        {{:ok, {bound_var}}} ->");
            for line in &cleaned_setup_lines[idx + 1..] {
                let _ = writeln!(out, "          {line}");
            }
            let call_invocation = if effective_args.is_empty() {
                format!("{module_path}.{function_name}()")
            } else {
                format!("{module_path}.{function_name}({effective_args})")
            };
            emit_error_assertion(out, "          ", &call_invocation, fixture, errors);
            let _ = writeln!(out, "      end");
        } else {
            for line in &cleaned_setup_lines {
                let _ = writeln!(out, "      {line}");
            }
            let call_invocation = if effective_args.is_empty() {
                format!("{module_path}.{function_name}()")
            } else {
                format!("{module_path}.{function_name}({effective_args})")
            };
            emit_error_assertion(out, "      ", &call_invocation, fixture, errors);
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
        emit_error_assertion(out, "      ", &call_invocation, fixture, errors);
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

    // NOTE: the trait-bridge `on_exit` teardown (see `emit_test_backend`'s doc comment in
    // stubs.rs) is already folded into `cleaned_setup_lines` above — right after the
    // trait-bridge marker split, before any of the three code paths below — so it is emitted
    // exactly once per test, in every path (validation-failure, expects-error, normal). Do
    // not re-emit `teardown_block` here; doing so previously produced a duplicate
    // `on_exit(fn -> ... end)` line per register_*_trait_bridge test.

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
    // A `returns_void` call whose `returns_result` is also true is the one case where this is
    // safe despite a `not_error` assertion being present: rustler encodes a Rust `()` success
    // payload as `nil`, so `render_assertion`'s `not_error` arm renders nothing for it
    // (asserting non-nil there would fail every successful call) — the `{:ok, result} =
    // call(...)` match emitted below is the real check. `result` would otherwise be bound and
    // never referenced. A `returns_void` call with `returns_result: false` (a bare-atom
    // fallible NIF, no tuple to match on) is NOT this case: the call-emission branch below
    // binds and asserts `result` directly, so it must stay referenced. A fixture whose only
    // assertion is `not_error` on a bare `Option<T>` result is the same "renders nothing"
    // shape again: `not_error_may_assert_presence` (computed above) is `false` there too, so
    // `render_assertion` renders nothing and `result` would otherwise go unreferenced. ~keep
    let all_not_error = fixture
        .assertions
        .iter()
        .all(|assertion| assertion.assertion_type == "not_error");
    let void_tuple_case = call_config.returns_void && returns_result;
    let not_error_renders_nothing =
        all_not_error && (void_tuple_case || (!call_config.returns_void && !not_error_may_assert_presence));
    let actual_result_var = if !is_streaming && (fixture.assertions.is_empty() || not_error_renders_nothing) {
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
        // Non-Result function returns value directly (e.g., bool, String). This is also the
        // shape of a fallible NIF with no `Result` wrapper for rustler to auto-tuple: it
        // encodes success/failure as the bare atoms `:ok`/`:error` directly (rustler
        // convention: `Ok(_) => atom("ok")`, `Err(_) => atom("error")`). There is no `{:ok, _}`
        // tuple to match on in that case, so unlike the branch above nothing raises a
        // `MatchError` on failure — `render_assertion`'s `not_error` arm emits the real
        // `assert result == :ok` check for it below, once assertions render. ~keep
        let _ = writeln!(out, "      {actual_result_var} = {call_invocation}");
    }

    // For streaming fixtures, drain the stream into a list before asserting.
    if is_streaming
        && let Some(collect) = crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet(
            "elixir",
            &result_var,
            chunks_var,
        )
    {
        let _ = writeln!(out, "      {collect}");
    }

    let mut assertions_body = String::new();
    for assertion in &fixture.assertions {
        render_assertion(
            &mut assertions_body,
            assertion,
            if is_streaming { chunks_var } else { &result_var },
            field_resolver,
            &module_path,
            e2e_config.effective_fields_enum(call_config),
            resolved_enum_fields_ref,
            result_is_simple,
            is_streaming,
            call_config.returns_void,
            returns_result,
            not_error_may_assert_presence,
        );
    }
    // A fixture that declared at least one assertion but every one of them resolved
    // to a "skipped" comment (all its fields are unavailable on the result type) is
    // otherwise indistinguishable from a fixture with zero declared assertions — an
    // entirely comment-only, vacuously-passing test body. `not_error` already emits
    // a real `refute is_nil(...)` (see `render_assertion`'s `not_error` arm) whenever
    // that's safe, so this only fires on the remaining gap: real field assertions that
    // all got dropped. Elixir was the one backend in this defect class with no fallback
    // of any kind for that case — mirror python/php/typescript's
    // `apply_vacuous_assertion_fallback`. A fixture with genuinely zero declared
    // assertions is left untouched, matching every other backend's deliberate "just
    // call it" smoke-test contract.
    //
    // `not_error_result_is_option` gates this the same way it gates `render_assertion`'s
    // own `not_error` arm: this fallback emits the identical `refute is_nil(...)` idiom,
    // so it is exactly as unsafe on a bare `Option<T>` result -- reinjecting it here would
    // silently undo `not_error_presence::may_assert_presence`'s decision for a fixture
    // whose only assertion is `not_error` on such a call (assertions_body is empty
    // precisely because that decision correctly suppressed it, not because a field
    // assertion was dropped). ~keep
    apply_vacuous_assertion_fallback(
        &mut assertions_body,
        !fixture.assertions.is_empty(),
        &VacuousFallbackCall {
            is_streaming,
            chunks_binding: chunks_var,
            result_binding: &result_var,
            returns_void: call_config.returns_void,
            returns_result,
            not_error_result_is_option,
        },
    );
    crate::e2e::codegen::fail_on_unavailable_field_markers(
        &assertions_body,
        "elixir",
        &fixture.id,
        &fixture.assertions,
    );
    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&assertions_body, "elixir", &fixture.id);
    out.push_str(&assertions_body);

    if needs_api_key_skip {
        let _ = writeln!(out, "      end");
    }
    let _ = writeln!(out, "    end");
    let _ = writeln!(out, "  end");
}

/// The call-shape facts the fallback consults. Grouped into one value because the elixir
/// variant needs six of them and a flat list crossed clippy's `too_many_arguments` cap. ~keep
struct VacuousFallbackCall<'a> {
    is_streaming: bool,
    // ~keep Named `*_binding`, not `*_var`: these hold an ALREADY-resolved binding name (the
    // caller passes `call_config.effective_result_var()`), and a field literally named
    // `result_var` collides with the text guard in `core::config::e2e::raw_result_var_reads`,
    // which exists to catch reads of `CallConfig`'s raw field and cannot tell the two apart.
    chunks_binding: &'a str,
    result_binding: &'a str,
    returns_void: bool,
    returns_result: bool,
    not_error_result_is_option: bool,
}

/// When a fixture declares at least one assertion but the rendered body has no
/// executable statement — every field assertion resolved to a "skipped" comment —
/// inject a real assertion instead of leaving the test vacuous. `not_error` already
/// renders a real `refute is_nil(...)` on its own (see `render_assertion`'s
/// `not_error` arm), so this only fires on the remaining gap: declared field
/// assertions that all turned out unavailable. Fixtures that declare NO assertions
/// at all are left untouched — a deliberate "just call it" smoke test, matching
/// every other backend in this defect class. Reuses the exact `refute is_nil(...)`
/// idiom `not_error` already emits, on whichever variable the assertion loop itself
/// targeted (`chunks_var` for streaming fixtures, `result_var` otherwise), so a
/// streaming fixture whose only real assertions were non-streaming-virtual field
/// checks gets covered by this same fix. ~keep
fn apply_vacuous_assertion_fallback(
    assertions_body: &mut String,
    has_declared_assertions: bool,
    call: &VacuousFallbackCall<'_>,
) {
    let has_real_assertion = assertions_body.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with('#')
    });
    if !has_declared_assertions || has_real_assertion {
        return;
    }
    let fallback_var = if call.is_streaming { call.chunks_binding } else { call.result_binding };
    // ~keep A void call's binding is `nil` on success (rustler encodes Rust `()` that way), so
    // `refute is_nil(...)` would fail every successful call, not just an unsuccessful one. When
    // `call.returns_result` is also true, the `{:ok, result} = call(...)` match this fallback's
    // caller already emitted is the real check for the call — see `render_assertion`'s
    // `not_error` arm for the identical reasoning. When `call.returns_result` is false there is no
    // such tuple and no match to rely on (a bare-atom fallible NIF, rustler convention
    // `Ok(_) => atom("ok")` / `Err(_) => atom("error")`), so a real check is still owed.
    if call.returns_void {
        if !call.returns_result {
            let _ = writeln!(assertions_body, "      assert {fallback_var} == :ok");
        }
        return;
    }
    // ~keep Same unsafety `not_error_presence::may_assert_presence` protects against: a bare
    // `Option<T>` result may legitimately be `nil` on success, so `refute is_nil(...)` is wrong
    // here for exactly the same reason it would be wrong in `render_assertion`'s own `not_error`
    // arm, regardless of why `assertions_body` ended up empty.
    if call.not_error_result_is_option {
        return;
    }
    if call.returns_result {
        let _ = writeln!(assertions_body, "      refute is_nil({fallback_var})");
    } else {
        // No tuple was matched above either, so the bare success value could just as easily
        // have arrived as the `:error` sentinel — `is_nil` alone cannot catch that failure
        // shape. See `render_assertion`'s `not_error` arm for the identical reasoning.
        let _ = writeln!(assertions_body, "      refute {fallback_var} in [nil, :error]");
    }
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

#[cfg(test)]
mod dropped_field_marker_tests {
    use super::render_test_case;
    use crate::e2e::config::{CallConfig, E2eConfig};
    use crate::e2e::fixture::{Assertion, Fixture};
    use std::collections::{HashMap, HashSet};

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

    /// Regression test for alef task #81: Elixir's "skipped: field not available" comment
    /// must carry the exact marker text the shared `fail_on_unavailable_field_markers`
    /// mechanism (src/e2e/codegen/mod.rs) matches on, so arming
    /// `ALEF_E2E_STRICT_FIELD_AVAILABILITY` turns a dropped field assertion into a
    /// generation-time failure. The arming behaviour itself is proven in `mod.rs`'s
    /// `unavailable_field_marker_tests`; this test only pins the marker text Elixir emits
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
        render_test_case(
            &mut out,
            &fixture,
            &e2e_config,
            "",
            "",
            "",
            &[],
            None,
            None,
            &HashMap::new(),
            None,
            &HashSet::new(),
            &[],
            &[],
            &config,
            &type_defs,
            &[],
            &[],
        );

        assert!(
            out.contains("field 'nonexistent_field' not available on result type"),
            "got:\n{out}"
        );
    }

    /// Regression test for alef task #81 (vacuous-fallback gap): a fixture whose
    /// sole assertion drops (its field is unavailable) must still get a real
    /// `refute is_nil(...)` on the bound result, not an entirely comment-only body
    /// that vacuously passes. Mirrors typescript's
    /// `dropped_field_assertion_still_gets_a_real_fallback_assertion`. ~keep
    #[test]
    fn dropped_field_assertion_still_gets_a_real_fallback_assertion() {
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
        render_test_case(
            &mut out,
            &fixture,
            &e2e_config,
            "",
            "",
            "",
            &[],
            None,
            None,
            &HashMap::new(),
            None,
            &HashSet::new(),
            &[],
            &[],
            &config,
            &type_defs,
            &[],
            &[],
        );

        assert!(
            out.contains("refute is_nil(result)"),
            "expected a real fallback assertion on the bound result, got:\n{out}"
        );
        assert!(
            out.contains("{:ok, result} ="),
            "the result must be bound (not `_result`) once a real assertion references it, got:\n{out}"
        );
    }

    /// Positive control for the same fix: a fixture with genuinely zero declared
    /// assertions is left untouched (deliberate "just call it" smoke test). Mirrors
    /// typescript's `zero_declared_assertions_are_left_untouched`. ~keep
    #[test]
    fn zero_declared_assertions_are_left_untouched() {
        let mut fixture = make_fixture("process_smoke", "nonexistent_field");
        fixture.assertions = Vec::new();
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
        render_test_case(
            &mut out,
            &fixture,
            &e2e_config,
            "",
            "",
            "",
            &[],
            None,
            None,
            &HashMap::new(),
            None,
            &HashSet::new(),
            &[],
            &[],
            &config,
            &type_defs,
            &[],
            &[],
        );

        assert!(
            !out.contains("refute is_nil(result)"),
            "a fixture with zero declared assertions must stay vacuous, got:\n{out}"
        );
    }

    #[cfg(test)]
    #[path = "void_not_error_binding_tests.rs"]
    mod void_not_error_binding_tests;

    #[cfg(test)]
    #[path = "bare_atom_not_error_tests.rs"]
    mod bare_atom_not_error_tests;
}
