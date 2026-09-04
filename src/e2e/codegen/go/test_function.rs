//! Go per-fixture test rendering.

use crate::codegen::naming::{go_free_function_name, go_param_name, to_go_name};
use crate::e2e::escape::go_string_literal;
use crate::e2e::fixture::Fixture;
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::fmt::Write as FmtWrite;

use super::assertions::render_assertion;
use super::client;
use super::setup::build_args_and_setup;
use super::visitors::visitor_struct_name;
use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, skip_line};

/// Emit the message-or-type-name check for a fixture's declared `error` assertion value — or,
/// when the declared value names a real error variant Go cannot substantiate, the registered
/// skip instead of an assertion that can never pass.
///
/// No-op when nothing is declared, leaving the `expects_error` branch's output unchanged from
/// before this check existed.
///
/// ~keep Mirrors the Rust/Python backends' disjunction (see
/// `crate::e2e::codegen::declared_error_value`): fixture authors name either a message
/// substring (config-validation fixtures) or a type-name prefix (API-error fixtures) in
/// the assertion's value, never both conventions at once. Checking `err.Error()` OR
/// `fmt.Sprintf("%T", err)` lets this single code path serve both. Whether Go can satisfy the
/// second convention for a given variant is decided once by `declared_error_variant::classify`
/// — Go dispatches to a variant-named error only when `#[alef(error_code = N)]` is declared.
fn emit_declared_error_value_assertion(out: &mut String, fixture: &Fixture, errors: &[crate::core::ir::ErrorDef]) {
    match classify("go", fixture, errors) {
        DeclaredErrorAssertion::Undeclared => {}
        DeclaredErrorAssertion::Assert(declared) => {
            let expected = go_string_literal(declared);
            let _ = writeln!(out, "\tif err != nil {{");
            let _ = writeln!(
                out,
                "\t\tif !strings.Contains(err.Error(), {expected}) && !strings.Contains(fmt.Sprintf(\"%T\", err), \
                 {expected}) {{"
            );
            let _ = writeln!(
                out,
                "\t\t\tt.Errorf(\"expected error to match %s, got message=%q type=%T\", {expected}, err.Error(), \
                 err)"
            );
            let _ = writeln!(out, "\t\t}}");
            let _ = writeln!(out, "\t}}");
        }
        DeclaredErrorAssertion::Unsubstantiable(variant) => {
            let _ = writeln!(out, "{}", skip_line("\t", "//", variant, &fixture.id, "go"));
        }
    }
}

/// Map a trait name to its Clear* function name.
/// E.g., "DocumentExtractor" -> "ClearDocumentExtractors"
fn clear_function_for_trait(trait_name: &str) -> Option<String> {
    match trait_name {
        "DocumentExtractor" => Some("ClearDocumentExtractors".to_string()),
        "OcrBackend" => Some("ClearOcrBackends".to_string()),
        "PostProcessor" => Some("ClearPostProcessors".to_string()),
        "Validator" => Some("ClearValidators".to_string()),
        "EmbeddingBackend" => Some("ClearEmbeddingBackends".to_string()),
        "Renderer" => Some("ClearRenderers".to_string()),
        _ => None,
    }
}

/// Determine the trait type being registered from the function name.
/// E.g., "RegisterDocumentExtractor" or "register_document_extractor" -> "DocumentExtractor"
fn trait_from_register_function(fn_name: &str) -> Option<String> {
    let lower = fn_name.to_lowercase();
    match lower.as_str() {
        "registerdocumentextractor" | "register_document_extractor" => Some("DocumentExtractor".to_string()),
        "registerocrbackend" | "register_ocr_backend" => Some("OcrBackend".to_string()),
        "registerpostprocessor" | "register_post_processor" => Some("PostProcessor".to_string()),
        "registervalidator" | "register_validator" => Some("Validator".to_string()),
        "registerembeddingbackend" | "register_embedding_backend" => Some("EmbeddingBackend".to_string()),
        "registerrenderer" | "register_renderer" => Some("Renderer".to_string()),
        _ => None,
    }
}

/// Emit cleanup call for trait-bridge fixtures to avoid cgo finalizer panics.
fn emit_trait_bridge_cleanup(out: &mut String, fixture: &Fixture, base_function_name: &str, import_alias: &str) {
    if fixture.tags.contains(&"trait-bridge".to_string())
        && let Some(trait_type) = trait_from_register_function(base_function_name)
        && let Some(clear_fn) = clear_function_for_trait(&trait_type)
    {
        let _ = writeln!(out, "\tif err := {import_alias}.{clear_fn}(); err != nil {{");
        let _ = writeln!(out, "\t\tt.Logf(\"{clear_fn} cleanup failed: %v\", err)");
        let _ = writeln!(out, "\t}}");
    }
}

pub(super) struct GoTestFunctionContext<'a> {
    pub(super) import_alias: &'a str,
    pub(super) e2e_config: &'a crate::e2e::config::E2eConfig,
    pub(super) adapters: &'a [crate::core::config::AdapterConfig],
    pub(super) data_enum_names: &'a std::collections::HashSet<&'a str>,
    pub(super) config: &'a crate::core::config::ResolvedCrateConfig,
    pub(super) type_defs: &'a [crate::core::ir::TypeDef],
    pub(super) enums: &'a [crate::core::ir::EnumDef],
    pub(super) errors: &'a [crate::core::ir::ErrorDef],
    pub(super) functions: &'a [crate::core::ir::FunctionDef],
}

#[cfg(test)]
pub(super) fn render_test_function(out: &mut String, fixture: &Fixture, context: GoTestFunctionContext<'_>) {
    let facts = call_resolver::GoCrateResolverFacts::new(context.type_defs, context.enums, context.config);
    render_test_function_with_facts(out, fixture, context, &facts);
}

pub(super) fn render_test_function_with_facts(
    out: &mut String,
    fixture: &Fixture,
    context: GoTestFunctionContext<'_>,
    crate_facts: &call_resolver::GoCrateResolverFacts,
) {
    let GoTestFunctionContext {
        import_alias,
        e2e_config,
        adapters,
        data_enum_names,
        config,
        type_defs,
        enums,
        errors,
        functions,
    } = context;
    let fn_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;

    if fixture.http.is_some() {
        render_http_test_function(out, fixture);
        return;
    }

    if !fixture_has_go_callable(fixture, e2e_config) {
        let _ = writeln!(out, "func Test_{fn_name}(t *testing.T) {{");
        let _ = writeln!(out, "\t// {description}");
        let _ = writeln!(
            out,
            "\tt.Skip(\"non-HTTP fixture: Go binding does not expose a callable for the configured `[e2e.call]` function\")"
        );
        let _ = writeln!(out, "}}");
        return;
    }

    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let lang = call_resolver::LANG;
    let call_field_resolver =
        call_resolver::build_call_field_resolver_with_facts(e2e_config, call_config, functions, type_defs, crate_facts);
    let field_resolver = &call_field_resolver;
    let overrides = call_config.overrides.get(lang);
    let base_function_name = overrides
        .and_then(|o| o.function.as_deref())
        .unwrap_or(&call_config.function);
    let function_name = go_free_function_name(base_function_name, crate_facts.reserved_type_names());
    let result_var = call_config.effective_result_var();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs);
    let args = recipe.args;

    let returns_result = overrides
        .and_then(|o| o.returns_result)
        .unwrap_or(call_config.returns_result);
    let returns_void = call_config.returns_void;
    let result_is_simple = overrides.is_some_and(|o| o.result_is_simple)
        || call_config.result_is_simple
        || call_config
            .overrides
            .get("rust")
            .map(|o| o.result_is_simple)
            .unwrap_or(false);
    let result_is_array = overrides.is_some_and(|o| o.result_is_array) || call_config.result_is_array;
    let call_options_type = recipe.options_type.or_else(|| {
        e2e_config
            .call
            .overrides
            .get("go")
            .and_then(|o| o.options_type.as_deref())
    });
    let call_options_ptr = overrides.map(|o| o.options_ptr).unwrap_or_else(|| {
        e2e_config
            .call
            .overrides
            .get("go")
            .map(|o| o.options_ptr)
            .unwrap_or(false)
    });

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let validation_creation_failure = expects_error && fixture.resolved_category() == "validation";
    let client_factory = overrides.and_then(|o| o.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.client_factory.as_deref())
    });

    let (package_decls, mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        import_alias,
        call_options_type,
        fixture,
        call_options_ptr,
        validation_creation_failure,
        data_enum_names,
        config,
        type_defs,
        enums,
        false,
        // The test-file emitter threads no `CallIr`, so every argument keeps exactly the
        // rendering it had before the `TargetParams` seam existed. ~keep
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
    )
    // The only refusal `build_args_and_setup` can raise comes from the native-DTO literal
    // builder, which the `native_dtos = false` argument above disables — the generated test
    // suite unmarshals its values from JSON instead of spelling typed Go literals, so no
    // value here is ever rendered against a declared Go type. Unreachable, not swallowed. ~keep
    .expect("go test-function rendering does not use native DTO literals and cannot refuse");

    for decl in &package_decls {
        out.push_str(decl);
        if !decl.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }

    let mut visitor_opts_var: Option<String> = None;
    if fixture.visitor.is_some() {
        let struct_name = visitor_struct_name(&fixture.id);
        setup_lines.push(format!("visitor := &{struct_name}{{}}"));
        // A visitor has no standalone Go parameter — it is only reachable by assignment
        // onto an options struct, so without a resolvable options type there is nowhere
        // to attach it. This used to emit a `t.Skip(...)` body, which reads as an
        // author-intended skip in `go test` output but is really a config failure the
        // suite then reports green. The sanctioned opt-outs (`skip_languages`, checked in
        // `fixture_has_go_callable` above, and `fixture.skip` via `fixture_inclusion`)
        // have both already run by here, so reaching this point means the fixture is
        // genuinely required for Go. Fail generation instead of emitting a weaker test. ~keep
        let Some(opts_type) =
            call_options_type.or_else(|| crate::e2e::codegen::recipe::trait_bridge_options_type(config))
        else {
            panic!(
                "Go e2e generator: fixture `{}` declares a `visitor`, but neither its `[e2e.call]` config nor any `[[crates.trait_bridges]]` entry provides an `options_type` to attach it to; cannot generate a Go visitor test without a resolvable trait bridge options type",
                fixture.id
            );
        };
        let opts_var = "opts".to_string();
        setup_lines.push(format!("opts := &{import_alias}.{opts_type}{{}}"));
        setup_lines.push("opts.Visitor = visitor".to_string());
        visitor_opts_var = Some(opts_var);
    }

    let go_extra_args = recipe.extra_args.to_vec();
    let final_args = {
        let mut parts: Vec<String> = Vec::new();
        if !args_str.is_empty() {
            let processed_args = if let Some(ref opts_var) = visitor_opts_var {
                args_str.trim_end_matches(", nil").to_string() + ", " + opts_var
            } else {
                args_str
            };
            parts.push(processed_args);
        }
        parts.extend(go_extra_args);
        parts.join(", ")
    };

    let _ = writeln!(out, "func Test_{fn_name}(t *testing.T) {{");
    let _ = writeln!(out, "\t// {description}");

    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    if let Some(var) = api_key_var {
        if has_mock {
            let fixture_id = &fixture.id;
            let _ = writeln!(out, "\tapiKey := os.Getenv(\"{var}\")");
            let _ = writeln!(out, "\tvar baseURL *string");
            let _ = writeln!(out, "\tif apiKey != \"\" {{");
            let _ = writeln!(out, "\t\tt.Logf(\"{fixture_id}: using real API ({var} is set)\")");
            let _ = writeln!(out, "\t}} else {{");
            let _ = writeln!(out, "\t\tt.Logf(\"{fixture_id}: using mock server ({var} not set)\")");
            let _ = writeln!(
                out,
                "\t\tu := os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\""
            );
            let _ = writeln!(out, "\t\tbaseURL = &u");
            let _ = writeln!(out, "\t\tapiKey = \"test-key\"");
            let _ = writeln!(out, "\t}}");
        } else {
            let _ = writeln!(out, "\tapiKey := os.Getenv(\"{var}\")");
            let _ = writeln!(out, "\tif apiKey == \"\" {{");
            let _ = writeln!(out, "\t\tt.Skipf(\"{var} not set\")");
            let _ = writeln!(out, "\t}}");
        }
    }

    for line in &setup_lines {
        let _ = writeln!(out, "\t{line}");
    }

    let call_prefix = if let Some(factory) = client_factory {
        let factory_name = to_go_name(factory);
        let fixture_id = &fixture.id;
        let (api_key_expr, base_url_expr) = if has_mock && api_key_var.is_some() {
            ("apiKey".to_string(), "baseURL".to_string())
        } else if api_key_var.is_some() {
            ("apiKey".to_string(), "nil".to_string())
        } else if fixture.has_host_root_route() {
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
            let _ = writeln!(out, "\tmockURL := os.Getenv(\"{env_key}\")");
            let _ = writeln!(out, "\tif mockURL == \"\" {{");
            let _ = writeln!(
                out,
                "\t\tmockURL = os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\""
            );
            let _ = writeln!(out, "\t}}");
            ("\"test-key\"".to_string(), "&mockURL".to_string())
        } else {
            let _ = writeln!(
                out,
                "\tmockURL := os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\""
            );
            ("\"test-key\"".to_string(), "&mockURL".to_string())
        };
        let _ = writeln!(
            out,
            "\tclient, clientErr := {import_alias}.{factory_name}({api_key_expr}, {base_url_expr}, nil, nil, nil)"
        );
        let _ = writeln!(out, "\tif clientErr != nil {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"create client failed: %v\", clientErr)");
        let _ = writeln!(out, "\t}}");
        "client".to_string()
    } else {
        import_alias.to_string()
    };

    let binding_returns_error_pre = args
        .iter()
        .any(|a| matches!(a.arg_type.as_str(), "json_object" | "bytes"));
    let effective_returns_result_pre = returns_result || binding_returns_error_pre || client_factory.is_some();

    if expects_error {
        if effective_returns_result_pre && !returns_void {
            let _ = writeln!(out, "\t_, err := {call_prefix}.{function_name}({final_args})");
        } else {
            let _ = writeln!(out, "\terr := {call_prefix}.{function_name}({final_args})");
        }
        let _ = writeln!(out, "\tif err == nil {{");
        let _ = writeln!(out, "\t\tt.Errorf(\"expected an error, but call succeeded\")");
        let _ = writeln!(out, "\t}}");
        emit_declared_error_value_assertion(out, fixture, errors);
        crate::e2e::codegen::error_path_assertions::emit(out, fixture, "\t// ", "go");
        emit_trait_bridge_cleanup(out, fixture, base_function_name, import_alias);
        let _ = writeln!(out, "}}");
        return;
    }

    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());
    let fn_snake = function_name.to_snake_case();
    let base_snake = base_function_name.to_snake_case();
    let streaming_item_type = is_streaming
        .then(|| crate::e2e::codegen::recipe::streaming_item_type(call_config, adapters, &[&fn_snake, &base_snake]))
        .flatten();

    let has_usable_assertion = fixture.assertions.iter().any(|a| {
        if a.assertion_type == "not_error" || a.assertion_type == "error" {
            return false;
        }
        if a.assertion_type == "method_result" {
            return true;
        }
        match &a.field {
            Some(f) if !f.is_empty() => {
                if is_streaming && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) {
                    return true;
                }
                if result_is_simple {
                    return true;
                }
                field_resolver.is_valid_for_result(f)
            }
            _ => true,
        }
    });

    let binding_returns_error = args
        .iter()
        .any(|a| matches!(a.arg_type.as_str(), "json_object" | "bytes"));
    let effective_returns_result = returns_result || binding_returns_error || client_factory.is_some();

    // ~keep `returns_void` means the only return is `error`, where nil is success. It must
    // win over `result_is_simple`, or the branch below asserts the error is NON-nil and the
    // test passes only when the call fails.
    if !effective_returns_result && result_is_simple && !returns_void {
        // A fixture that declared an assertion but produced none usable here (e.g. its
        // only assertion is `not_error`, and this function has no error return to check
        // in the first place) used to be indistinguishable from a genuinely empty
        // fixture: both discarded the result to `_` and asserted nothing at all. Every
        // other backend in this defect class (python, php, ...) still emits a real
        // fallback check when the fixture declared *something* — mirror that here by
        // binding the result and asserting non-nil instead of discarding it, for the one
        // Go shape (`!effective_returns_result && result_is_simple`) that has no error
        // return to fall back on either. ~keep
        let declared_but_unusable = !has_usable_assertion && !fixture.assertions.is_empty();
        let result_binding = if has_usable_assertion || declared_but_unusable {
            result_var.to_string()
        } else {
            "_".to_string()
        };
        let assign_op = if result_binding == "_" { "=" } else { ":=" };
        let _ = writeln!(
            out,
            "\t{result_binding} {assign_op} {call_prefix}.{function_name}({final_args})"
        );
        if has_usable_assertion && result_binding != "_" {
            if result_is_array {
                let _ = writeln!(out, "\tvalue := {result_var}");
            } else {
                let only_nil_assertions = fixture
                    .assertions
                    .iter()
                    .filter(|a| a.field.as_ref().is_none_or(|f| f.is_empty()))
                    .filter(|a| !matches!(a.assertion_type.as_str(), "not_error" | "error"))
                    .all(|a| matches!(a.assertion_type.as_str(), "is_empty" | "is_null"));
                if !only_nil_assertions {
                    let result_is_ptr = overrides.map(|o| o.result_is_pointer).unwrap_or(true);
                    if result_is_ptr {
                        let _ = writeln!(out, "\tif {result_var} == nil {{");
                        let _ = writeln!(out, "\t\tt.Fatalf(\"expected non-nil result\")");
                        let _ = writeln!(out, "\t}}");
                        let _ = writeln!(out, "\tvalue := *{result_var}");
                    } else {
                        let _ = writeln!(out, "\tvalue := {result_var}");
                    }
                }
            }
        } else if declared_but_unusable {
            let result_is_ptr = overrides.map(|o| o.result_is_pointer).unwrap_or(true);
            if result_is_ptr {
                let _ = writeln!(out, "\tif {result_var} == nil {{");
                let _ = writeln!(out, "\t\tt.Fatalf(\"expected non-nil result\")");
                let _ = writeln!(out, "\t}}");
            } else {
                // No type-safe zero-value check exists generically for a non-pointer
                // result without knowing its concrete Go type; keep the binding (so a
                // future assertion can reference it) but consume it to satisfy
                // `-D unused_variables` rather than silently discarding to `_`.
                let _ = writeln!(out, "\t_ = {result_var}");
            }
        }
    } else if !effective_returns_result || returns_void {
        let _ = writeln!(out, "\terr := {call_prefix}.{function_name}({final_args})");
        let _ = writeln!(out, "\tif err != nil {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"call failed: %v\", err)");
        let _ = writeln!(out, "\t}}");
        emit_trait_bridge_cleanup(out, fixture, base_function_name, import_alias);
        let _ = writeln!(out, "}}");
        return;
    } else {
        let result_binding = if is_streaming {
            "stream".to_string()
        } else if has_usable_assertion {
            result_var.to_string()
        } else {
            "_".to_string()
        };
        let _ = writeln!(
            out,
            "\t{result_binding}, err := {call_prefix}.{function_name}({final_args})"
        );
        let _ = writeln!(out, "\tif err != nil {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"call failed: %v\", err)");
        let _ = writeln!(out, "\t}}");
        if is_streaming {
            if let Some(streaming_item_type) = streaming_item_type {
                let _ = writeln!(out, "\tvar chunks []{import_alias}.{streaming_item_type}");
                let _ = writeln!(out, "\tfor chunk := range stream {{");
                let _ = writeln!(out, "\t\tchunks = append(chunks, chunk)");
                let _ = writeln!(out, "\t}}");
            } else {
                let _ = writeln!(
                    out,
                    "\t// skipped: streaming fixture requires adapter item_type for Go e2e codegen"
                );
            }
        }
        if result_is_simple && has_usable_assertion && result_binding != "_" {
            if result_is_array {
                let _ = writeln!(out, "\tvalue := {}", result_var);
            } else {
                let only_nil_assertions = fixture
                    .assertions
                    .iter()
                    .filter(|a| a.field.as_ref().is_none_or(|f| f.is_empty()))
                    .filter(|a| !matches!(a.assertion_type.as_str(), "not_error" | "error"))
                    .all(|a| matches!(a.assertion_type.as_str(), "is_empty" | "is_null"));
                if !only_nil_assertions {
                    let result_is_ptr = overrides.map(|o| o.result_is_pointer).unwrap_or(true);
                    if result_is_ptr {
                        let _ = writeln!(out, "\tif {} == nil {{", result_var);
                        let _ = writeln!(out, "\t\tt.Fatalf(\"expected non-nil result\")");
                        let _ = writeln!(out, "\t}}");
                        let _ = writeln!(out, "\tvalue := *{}", result_var);
                    } else {
                        let _ = writeln!(out, "\tvalue := {}", result_var);
                    }
                }
            }
        }
    }

    let result_is_ptr = overrides.map(|o| o.result_is_pointer).unwrap_or(true);
    let has_deref_value = if result_is_simple && has_usable_assertion && !result_is_array && result_is_ptr {
        let only_nil_assertions = fixture
            .assertions
            .iter()
            .filter(|a| a.field.as_ref().is_none_or(|f| f.is_empty()))
            .filter(|a| !matches!(a.assertion_type.as_str(), "not_error" | "error"))
            .all(|a| matches!(a.assertion_type.as_str(), "is_empty" | "is_null"));
        !only_nil_assertions
    } else if result_is_simple && has_usable_assertion && result_is_ptr {
        true
    } else {
        result_is_simple && has_usable_assertion
    };

    let effective_result_var = if has_deref_value {
        "value".to_string()
    } else {
        result_var.to_string()
    };

    // Skipped entirely for streaming fixtures: `effective_result_var` (e.g. "result"/"value")
    // is never bound in the streaming arm above -- only `stream`/`chunks` are -- so
    // unconditionally building an `<local> := result.<field>` binding here referenced an
    // undeclared variable (`undefined: result`) whenever a streaming fixture asserted a
    // string-typed optional field. Streaming-virtual-field assertions (`finish_reason`,
    // `tool_calls`, ...) are rendered separately by `render_assertion`'s streaming arm. ~keep
    let mut optional_locals: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for assertion in &fixture.assertions {
        if is_streaming {
            break;
        }
        if let Some(f) = &assertion.field
            && !f.is_empty()
        {
            if !result_is_simple && !field_resolver.is_valid_for_result(f) {
                continue;
            }
            // Bracket-wildcard paths are rendered by the any-element scan in
            // `render_assertion`, which ignores `optional_locals`. Binding a local here
            // would leave it declared and never used — a compile error in Go. ~keep
            if f.contains("[].") {
                continue;
            }
            let resolved = field_resolver.resolve(f);
            // ~keep This site's `None` fallback deliberately DIFFERS from
            // `assertion_field_shape.rs`'s `unwrap_or(false)`, and the difference is load-bearing:
            // harmonizing the two was tried and reverted. There, `false` is right -- it prevents
            // the `len(*result.Chunks)` deref of a plain `[]Chunk`. Here, `false` is wrong -- it
            // DROPS a required deref, emitting `string(result.Content)` for a `*string` and
            // failing `test_go_plain_optional_string_uses_string_deref_not_text_accessor` and
            // `test_go_display_as_text_optional_uses_text_accessor_not_string_deref`. The two
            // sites ask the same question about different path shapes, so neither fallback is
            // universally safe. The real fix is to make `target_field_is_pointer` total for
            // data-enum crossings so `None` stops happening; until then do not "unify" these.
            let is_pointer = field_resolver
                .target_field_is_pointer(resolved)
                .unwrap_or_else(|| field_resolver.is_optional(resolved) && !field_resolver.is_array(resolved));
            if is_pointer && !optional_locals.contains_key(f.as_str()) {
                let is_string_field = assertion.value.as_ref().is_some_and(|v| v.is_string());
                let is_array_field = field_resolver.is_array(resolved);
                // Both plain-string and display_as_text optional fields only need a
                // local binding when the assertion value is a string. Non-string
                // assertions (numeric, boolean) and array fields are handled elsewhere.
                if !is_string_field || is_array_field {
                    continue;
                }
                let is_dat = field_resolver.is_display_as_text(f);
                let field_expr = field_resolver.accessor(f, "go", &effective_result_var);
                let local_var = go_param_name(&resolved.replace(['.', '[', ']'], "_"));
                if field_resolver.has_map_access(f) {
                    let _ = writeln!(out, "\t{local_var} := {field_expr}");
                } else if is_dat {
                    // Non-String inner type with a .Text() accessor: calling
                    // `string(*field_expr)` would fail to compile because the pointer
                    // element is not a primitive string. Use the text accessor instead.
                    let _ = writeln!(out, "\tvar {local_var} string");
                    let _ = writeln!(out, "\tif {field_expr} != nil {{");
                    let _ = writeln!(out, "\t\t{local_var} = {field_expr}.Text()");
                    let _ = writeln!(out, "\t}}");
                } else {
                    let _ = writeln!(out, "\tvar {local_var} string");
                    let _ = writeln!(out, "\tif {field_expr} != nil {{");
                    let _ = writeln!(out, "\t\t{local_var} = string(*{field_expr})");
                    let _ = writeln!(out, "\t}}");
                }
                optional_locals.insert(f.clone(), local_var);
            }
        }
    }

    // Fields this fixture compares numerically elsewhere (`equals`/`greater_than[_or_equal]`/
    // `less_than[_or_equal]` against a JSON number) — proof the field is a scalar number, not
    // a sized Go type. `not_empty` on the same field cannot call `len()` (it would not compile
    // against e.g. `float64`), so it consults this set to skip straight to a no-op.
    let numeric_scalar_fields: std::collections::HashSet<&str> = fixture
        .assertions
        .iter()
        .filter(|a| {
            matches!(
                a.assertion_type.as_str(),
                "equals" | "greater_than" | "greater_than_or_equal" | "less_than" | "less_than_or_equal"
            ) && a.value.as_ref().is_some_and(serde_json::Value::is_number)
        })
        .filter_map(|a| a.field.as_deref())
        .collect();

    // Fields this fixture already asserts presence on via `not_empty` -- a guarded count
    // assertion (`count_min`/`count_equals`) on the same field can rely on that sibling
    // to fail the test on nil, instead of adding its own duplicate failing branch.
    let presence_checked_fields: std::collections::HashSet<&str> = fixture
        .assertions
        .iter()
        .filter(|a| a.assertion_type == "not_empty")
        .filter_map(|a| a.field.as_deref())
        .collect();

    // Call-invariant across every assertion in this fixture's loop below -- built once
    // rather than threaded as separate parameters through each `render_assertion` call.
    let assertion_context = super::assertions::AssertionRenderContext {
        effective_result_var: &effective_result_var,
        import_alias,
        field_resolver,
        optional_locals: &optional_locals,
        numeric_scalar_fields: &numeric_scalar_fields,
        presence_checked_fields: &presence_checked_fields,
        result_is_simple,
        result_is_array,
        is_streaming,
        streaming_item_type,
    };

    // `out` accumulates every fixture's rendered function in this file (see the
    // caller in `test_file.rs`), so the strict-availability scan below must only
    // look at the text this fixture's own assertion loop appends — scanning the
    // whole buffer would misattribute an earlier fixture's skip comment to this
    // fixture's id.
    let assertions_start = out.len();
    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field
            && !f.is_empty()
            && !optional_locals.contains_key(f.as_str())
        {
            let parts: Vec<&str> = f.split('.').collect();
            let mut guard_expr: Option<String> = None;
            // The dotted ancestor path (raw fixture spelling, e.g. `results[0].metadata.format`)
            // that produced `guard_expr` -- kept alongside it so the wrap below can check
            // `presence_checked_fields` (built from the SAME raw spelling) for a sibling
            // `not_empty` on this exact ancestor. ~keep
            let mut guard_ancestor_prefix: Option<String> = None;
            for i in 1..parts.len() {
                let prefix = parts[..i].join(".");
                let resolved_prefix = field_resolver.resolve(&prefix);
                if field_resolver.is_optional(resolved_prefix) {
                    let guard_prefix = if let Some(bracket_pos) = resolved_prefix.rfind('[') {
                        let suffix = &resolved_prefix[bracket_pos + 1..];
                        let is_numeric_index = suffix.trim_end_matches(']').chars().all(|c| c.is_ascii_digit());
                        if is_numeric_index {
                            &resolved_prefix[..bracket_pos]
                        } else {
                            resolved_prefix
                        }
                    } else {
                        resolved_prefix
                    };
                    let accessor = field_resolver.accessor(guard_prefix, "go", &effective_result_var);
                    guard_expr = Some(accessor);
                    guard_ancestor_prefix = Some(prefix);
                    break;
                }
            }
            if let Some(guard) = guard_expr {
                if field_resolver.is_valid_for_result(f) {
                    let is_struct_value = !guard.contains('[') && !guard.contains('(') && !guard.contains("map");
                    if is_struct_value {
                        render_assertion(out, &assertion_context, assertion);
                        continue;
                    }
                    let mut nil_buf = String::new();
                    render_assertion(&mut nil_buf, &assertion_context, assertion);
                    // A sibling `not_empty` on this EXACT ancestor path already fails the test
                    // when the ancestor is nil, so an `else` here would only duplicate that
                    // failure -- stay guard-only, matching `render_count_assertion`'s and
                    // `render_length_assertion`'s identical sibling-presence exemption. An empty
                    // `nil_buf` means the wrapped assertion type renders nothing on its own
                    // (e.g. `not_error`/`error`, which never carry a `field` in practice, or a
                    // dispatcher that already emitted its own failure elsewhere) -- adding a
                    // failing `else` around nothing would fail the test on a no-op, so that case
                    // also stays guard-only. ~keep
                    let sibling_covers_ancestor = guard_ancestor_prefix
                        .as_deref()
                        .is_some_and(|p| presence_checked_fields.contains(p));
                    let _ = writeln!(out, "\tif {guard} != nil {{");
                    for line in nil_buf.lines() {
                        let _ = writeln!(out, "\t{line}");
                    }
                    if sibling_covers_ancestor || nil_buf.trim().is_empty() {
                        let _ = writeln!(out, "\t}}");
                    } else {
                        let _ = writeln!(out, "\t}} else {{");
                        let _ = writeln!(out, "\t\tt.Errorf(\"expected %s to be present, got nil\", `{guard}`)");
                        let _ = writeln!(out, "\t}}");
                    }
                } else {
                    render_assertion(out, &assertion_context, assertion);
                }
                continue;
            }
        }
        render_assertion(out, &assertion_context, assertion);
    }
    crate::e2e::codegen::fail_on_unavailable_field_markers(
        &out[assertions_start..],
        "go",
        &fixture.id,
        &fixture.assertions,
    );
    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&out[assertions_start..], "go", &fixture.id);

    emit_trait_bridge_cleanup(out, fixture, base_function_name, import_alias);
    let _ = writeln!(out, "}}");
}

fn render_http_test_function(out: &mut String, fixture: &Fixture) {
    client::http_call::render_http_test(out, &GoTestClientRenderer, fixture);
}

struct GoTestClientRenderer;

impl client::TestClientRenderer for GoTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "go"
    }

    fn sanitize_test_name(&self, id: &str) -> String {
        id.to_upper_camel_case()
    }

    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let _ = writeln!(out, "func Test_{fn_name}(t *testing.T) {{");
        let _ = writeln!(out, "\t// {description}");
        if let Some(reason) = skip_reason {
            let escaped = go_string_literal(reason);
            let _ = writeln!(out, "\tt.Skip({escaped})");
        }
    }

    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "}}");
    }

    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();
        let path = ctx.path;

        let _ = writeln!(out, "\tbaseURL := os.Getenv(\"SUT_URL\")");
        let _ = writeln!(out, "\tif baseURL == \"\" {{");
        let _ = writeln!(out, "\t\tbaseURL = \"http://127.0.0.1:8012\"");
        let _ = writeln!(out, "\t}}");

        let body_expr = if let Some(body) = ctx.body {
            // A string body is a raw request body (e.g. urlencoded form data) and
            // must be emitted verbatim; JSON-serializing it would wrap it in quotes
            // (`"username=ab"`), which the server cannot parse. Non-string bodies are
            // serialized to JSON.
            let raw = match body {
                serde_json::Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            let escaped = go_string_literal(&raw);
            format!("strings.NewReader({})", escaped)
        } else {
            "strings.NewReader(\"\")".to_string()
        };

        let _ = writeln!(out, "\tbody := {body_expr}");
        let _ = writeln!(
            out,
            "\treq, err := http.NewRequest(\"{method}\", baseURL+\"{path}\", body)"
        );
        let _ = writeln!(out, "\tif err != nil {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"new request failed: %v\", err)");
        let _ = writeln!(out, "\t}}");

        if ctx.body.is_some() {
            let content_type = ctx.content_type.unwrap_or("application/json");
            let _ = writeln!(out, "\treq.Header.Set(\"Content-Type\", \"{content_type}\")");
        }

        let mut header_names: Vec<&String> = ctx.headers.keys().collect();
        header_names.sort();
        for name in header_names {
            let value = &ctx.headers[name];
            let escaped_name = go_string_literal(name);
            let escaped_value = go_string_literal(value);
            let _ = writeln!(out, "\treq.Header.Set({escaped_name}, {escaped_value})");
        }

        if !ctx.cookies.is_empty() {
            let mut cookie_names: Vec<&String> = ctx.cookies.keys().collect();
            cookie_names.sort();
            for name in cookie_names {
                let value = &ctx.cookies[name];
                let escaped_name = go_string_literal(name);
                let escaped_value = go_string_literal(value);
                let _ = writeln!(
                    out,
                    "\treq.AddCookie(&http.Cookie{{Name: {escaped_name}, Value: {escaped_value}}})"
                );
            }
        }

        let _ = writeln!(out, "\tnoRedirectClient := &http.Client{{");
        let _ = writeln!(
            out,
            "\t\tCheckRedirect: func(req *http.Request, via []*http.Request) error {{"
        );
        let _ = writeln!(out, "\t\t\treturn http.ErrUseLastResponse");
        let _ = writeln!(out, "\t\t}},");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\tresp, err := noRedirectClient.Do(req)");
        let _ = writeln!(out, "\tif err != nil {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"request failed: %v\", err)");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\tdefer func() {{ _ = resp.Body.Close() }}()");

        let _ = writeln!(out, "\tbodyBytes, err := io.ReadAll(resp.Body)");
        let _ = writeln!(out, "\tif err != nil {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"read body failed: %v\", err)");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\t_ = bodyBytes");
    }

    fn render_assert_status(&self, out: &mut String, _response_var: &str, status: u16) {
        let _ = writeln!(out, "\tif resp.StatusCode != {status} {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"status: got %d want {status}\", resp.StatusCode)");
        let _ = writeln!(out, "\t}}");
    }

    fn render_assert_header(&self, out: &mut String, _response_var: &str, name: &str, expected: &str) {
        if matches!(expected, "<<absent>>" | "<<present>>" | "<<uuid>>") {
            return;
        }
        if name.eq_ignore_ascii_case("connection") {
            return;
        }
        let escaped_name = go_string_literal(name);
        let escaped_value = go_string_literal(expected);
        let _ = writeln!(
            out,
            "\tif !strings.Contains(resp.Header.Get({escaped_name}), {escaped_value}) {{"
        );
        let _ = writeln!(
            out,
            "\t\tt.Fatalf(\"header %s mismatch: got %q want to contain %q\", {escaped_name}, resp.Header.Get({escaped_name}), {escaped_value})"
        );
        let _ = writeln!(out, "\t}}");
    }

    fn render_assert_json_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        match expected {
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                let json_str = serde_json::to_string(expected).unwrap_or_default();
                let escaped = go_string_literal(&json_str);
                let _ = writeln!(out, "\tvar got any");
                let _ = writeln!(out, "\tvar want any");
                let _ = writeln!(out, "\tif err := json.Unmarshal(bodyBytes, &got); err != nil {{");
                let _ = writeln!(out, "\t\tt.Fatalf(\"json unmarshal got: %v\", err)");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(
                    out,
                    "\tif err := json.Unmarshal([]byte({escaped}), &want); err != nil {{"
                );
                let _ = writeln!(out, "\t\tt.Fatalf(\"json unmarshal want: %v\", err)");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\tif !reflect.DeepEqual(got, want) {{");
                let _ = writeln!(out, "\t\tt.Fatalf(\"body mismatch: got %v want %v\", got, want)");
                let _ = writeln!(out, "\t}}");
            }
            serde_json::Value::String(s) => {
                let escaped = go_string_literal(s);
                let _ = writeln!(out, "\twant := {escaped}");
                let _ = writeln!(out, "\tif strings.TrimSpace(string(bodyBytes)) != want {{");
                let _ = writeln!(out, "\t\tt.Fatalf(\"body: got %q want %q\", string(bodyBytes), want)");
                let _ = writeln!(out, "\t}}");
            }
            other => {
                let escaped = go_string_literal(&other.to_string());
                let _ = writeln!(out, "\twant := {escaped}");
                let _ = writeln!(out, "\tif strings.TrimSpace(string(bodyBytes)) != want {{");
                let _ = writeln!(out, "\t\tt.Fatalf(\"body: got %q want %q\", string(bodyBytes), want)");
                let _ = writeln!(out, "\t}}");
            }
        }
    }

    fn render_assert_partial_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let _ = writeln!(out, "\tvar _partialGot map[string]any");
            let _ = writeln!(
                out,
                "\tif err := json.Unmarshal(bodyBytes, &_partialGot); err != nil {{"
            );
            let _ = writeln!(out, "\t\tt.Fatalf(\"json unmarshal partial: %v\", err)");
            let _ = writeln!(out, "\t}}");
            for (key, val) in obj {
                let escaped_key = go_string_literal(key);
                let json_val = serde_json::to_string(val).unwrap_or_default();
                let escaped_val = go_string_literal(&json_val);
                let _ = writeln!(out, "\t{{");
                let _ = writeln!(out, "\t\tvar _wantVal any");
                let _ = writeln!(
                    out,
                    "\t\tif err := json.Unmarshal([]byte({escaped_val}), &_wantVal); err != nil {{"
                );
                let _ = writeln!(out, "\t\t\tt.Fatalf(\"json unmarshal partial want: %v\", err)");
                let _ = writeln!(out, "\t\t}}");
                let _ = writeln!(
                    out,
                    "\t\tif !reflect.DeepEqual(_partialGot[{escaped_key}], _wantVal) {{"
                );
                let _ = writeln!(
                    out,
                    "\t\t\tt.Fatalf(\"partial body field {key}: got %v want %v\", _partialGot[{escaped_key}], _wantVal)"
                );
                let _ = writeln!(out, "\t\t}}");
                let _ = writeln!(out, "\t}}");
            }
        }
    }

    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        _response_var: &str,
        errors: &[crate::e2e::fixture::ValidationErrorExpectation],
    ) {
        let _ = writeln!(out, "\tvar _veBody map[string]any");
        let _ = writeln!(out, "\tif err := json.Unmarshal(bodyBytes, &_veBody); err != nil {{");
        let _ = writeln!(out, "\t\tt.Fatalf(\"json unmarshal validation errors: %v\", err)");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\t_veErrors, _ := _veBody[\"errors\"].([]any)");
        for ve in errors {
            let escaped_msg = go_string_literal(&ve.msg);
            let _ = writeln!(out, "\t{{");
            let _ = writeln!(out, "\t\t_found := false");
            let _ = writeln!(out, "\t\tfor _, _e := range _veErrors {{");
            let _ = writeln!(out, "\t\t\tif _em, ok := _e.(map[string]any); ok {{");
            let _ = writeln!(
                out,
                "\t\t\t\tif _msg, ok := _em[\"msg\"].(string); ok && strings.Contains(_msg, {escaped_msg}) {{"
            );
            let _ = writeln!(out, "\t\t\t\t\t_found = true");
            let _ = writeln!(out, "\t\t\t\t\tbreak");
            let _ = writeln!(out, "\t\t\t\t}}");
            let _ = writeln!(out, "\t\t\t}}");
            let _ = writeln!(out, "\t\t}}");
            let _ = writeln!(out, "\t\tif !_found {{");
            let _ = writeln!(
                out,
                "\t\t\tt.Fatalf(\"validation error with msg containing %q not found in errors\", {escaped_msg})"
            );
            let _ = writeln!(out, "\t\t}}");
            let _ = writeln!(out, "\t}}");
        }
    }
}

#[path = "test_function/call_resolver.rs"]
pub(super) mod call_resolver;
pub(super) use call_resolver::fixture_has_go_callable;

#[cfg(test)]
#[path = "test_function/declared_error_value_tests.rs"]
mod declared_error_value_tests;
