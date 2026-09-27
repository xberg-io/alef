//! Gleam fixture test-case renderer.

use crate::e2e::codegen::call_ir::resolve_declared_result_type;
use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, skip_line};
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{escape_gleam, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use heck::ToSnakeCase;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;

use super::args::build_args_and_setup;
use super::assertions::render_assertion;

/// Emit an error assertion for a call expression, closing the enclosing test function.
///
/// ~keep `string.inspect/1` renders any Gleam value as text (a String reason's quoted
/// content, or a custom error type's constructor name and fields), so a single substring
/// check against it enforces the same message-OR-type disjunction the other language
/// backends apply explicitly — for the fixtures `declared_error_variant::classify` recognises
/// as message-style. A value naming a real `ErrorVariant` renders the registered skip instead:
/// every Gleam binding rides on the same Rustler NIF glue that stringifies the reason before
/// `string.inspect/1` ever sees it, so a nominally-typed external signature never actually
/// carries a constructor name across the boundary.
fn emit_error_assertion(out: &mut String, call_expr: &str, fixture: &Fixture, errors: &[crate::core::ir::ErrorDef]) {
    match classify("gleam", fixture, errors) {
        DeclaredErrorAssertion::Undeclared => {
            let _ = writeln!(out, "  {call_expr} |> should.be_error()");
        }
        DeclaredErrorAssertion::Assert(value) => {
            let escaped = escape_gleam(value);
            let _ = writeln!(out, "  let __result = {call_expr}");
            let _ = writeln!(out, "  let assert Error(__reason) = __result");
            let _ = writeln!(
                out,
                "  should.be_true(string.contains(string.inspect(__reason), \"{escaped}\"))"
            );
        }
        DeclaredErrorAssertion::Unsubstantiable(variant) => {
            let _ = writeln!(out, "  let __result = {call_expr}");
            let _ = writeln!(out, "  let assert Error(_) = __result");
            let _ = writeln!(out, "  {}", skip_line("", "//", variant, &fixture.id, "gleam"));
        }
    }
    let _ = writeln!(out, "}}");
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_case(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    module_path: &str,
    _function_name: &str,
    _result_var: &str,
    _args: &[crate::e2e::config::ArgMapping],
    element_constructors: &[crate::core::config::GleamElementConstructor],
    json_object_wrapper: Option<&str>,
    ir: crate::e2e::codegen::call_ir::CallIr<'_>,
    enums: &[crate::core::ir::EnumDef],
    errors: &[crate::core::ir::ErrorDef],
) {
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let lang = "gleam";
    let call_overrides = call_config.overrides.get(lang);

    let mut effective_enum_fields: HashSet<String> = e2e_config.effective_fields_enum(call_config).clone();
    if let Some(o) = call_overrides {
        for k in o.enum_fields.keys() {
            effective_enum_fields.insert(k.clone());
        }
        for k in o.assert_enum_fields.keys() {
            effective_enum_fields.insert(k.clone());
        }
    }

    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        e2e_config.effective_fields_method_calls(call_config),
    )
    .with_enum_fields(effective_enum_fields)
    .with_ir_enum_map(
        FieldResolver::ir_enum_fields(ir.type_defs, enums),
        resolve_declared_result_type(call_config, lang, ir),
    );
    let field_resolver = &call_field_resolver;
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    let client_factory: Option<String> = call_overrides
        .and_then(|o| o.client_factory.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get(lang)
                .and_then(|o| o.client_factory.as_deref())
        })
        .map(|s| s.to_string());
    let client_factory_trailing_args: Vec<String> = call_overrides
        .map(|o| o.client_factory_trailing_args.clone())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            e2e_config
                .call
                .overrides
                .get(lang)
                .map(|o| o.client_factory_trailing_args.clone())
                .unwrap_or_default()
        });
    let extra_args: Vec<String> = call_overrides.map(|o| o.extra_args.clone()).unwrap_or_default();
    let result_var = call_config.effective_result_var();
    let args = fixture.resolved_args(call_config);

    let raw_name = sanitize_ident(&fixture.id);
    let stripped = raw_name.trim_start_matches(|c: char| c == '_' || c.is_ascii_digit());
    let test_name = if stripped.is_empty() {
        raw_name.as_str()
    } else {
        stripped
    };
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let options_type: Option<&str> = call_overrides.and_then(|o| o.options_type.as_deref());
    let options_via: &str = call_overrides
        .and_then(|o| o.options_via.as_deref())
        .unwrap_or("default");

    let test_documents_path = e2e_config.test_documents_relative_from(0);
    let build_result = build_args_and_setup(
        &fixture.input,
        args,
        &fixture.id,
        &test_documents_path,
        element_constructors,
        json_object_wrapper,
        module_path,
        &extra_args,
        options_type,
        options_via,
        fixture.preserve_input_urls,
        crate::e2e::codegen::call_ir::TargetParams::resolve(call_config, lang, ir),
        ir.type_defs,
        enums,
    );

    let _ = writeln!(out, "// {description}");
    let _ = writeln!(out, "pub fn {test_name}_test() {{");

    let (setup_lines, args_str) = match build_result {
        Ok(built) => built,
        Err(reason) => {
            let _ = writeln!(out, "  // skipped: {reason}");
            let _ = writeln!(out, "  Nil");
            let _ = writeln!(out, "}}");
            return;
        }
    };

    for line in &setup_lines {
        let _ = writeln!(out, "  {line}");
    }

    let call_prefix = if let Some(ref factory) = client_factory {
        let factory_snake = factory.to_snake_case();
        let trailing = if client_factory_trailing_args.is_empty() {
            String::new()
        } else {
            format!(", {}", client_factory_trailing_args.join(", "))
        };
        let base_url_expr = args
            .iter()
            .find(|a| a.arg_type == "mock_url")
            .map(|_a| {
                format!("let base_url__ = case envoy.get(\"MOCK_SERVER_URL\") {{ Ok(u) -> u Error(_) -> \"http://localhost:8080\" }}\n  let assert Ok(client) = {module_path}.{factory_snake}(\"test-key\", option.Some(base_url__){trailing})\n  let _ = client")
            })
            .unwrap_or_else(|| {
                format!("let assert Ok(client) = {module_path}.{factory_snake}(\"test-key\", option.None{trailing})\n  let _ = client")
            });
        for l in base_url_expr.lines() {
            let _ = writeln!(out, "  {l}");
        }
        let full_args = if args_str.is_empty() {
            "client".to_string()
        } else {
            format!("client, {args_str}")
        };
        if expects_error {
            let call_expr = format!("{module_path}.{function_name}({full_args})");
            emit_error_assertion(out, &call_expr, fixture, errors);
            return;
        }
        let _ = writeln!(out, "  let {result_var} = {module_path}.{function_name}({full_args})");
        None
    } else {
        if expects_error {
            let call_expr = format!("{module_path}.{function_name}({args_str})");
            emit_error_assertion(out, &call_expr, fixture, errors);
            return;
        }
        let _ = writeln!(out, "  let {result_var} = {module_path}.{function_name}({args_str})");
        Some(())
    };
    let _ = call_prefix;
    let _ = writeln!(out, "  {result_var} |> should.be_ok()");
    let _ = writeln!(out, "  let assert Ok(r) = {result_var}");

    let result_is_array = call_config.result_is_array || call_config.result_is_vec;
    let result_is_simple = call_overrides.is_some_and(|o| o.result_is_simple) || call_config.result_is_simple;
    let pkg_module = e2e_config
        .resolve_package("gleam")
        .as_ref()
        .and_then(|p| p.name.as_ref())
        .cloned()
        .unwrap_or_else(|| module_path.split('.').next().unwrap_or(module_path).to_string());

    // `out` accumulates every fixture's rendered test case in this file (see the
    // caller in `test_file.rs`), so the strict-availability scan below must only
    // look at the text this fixture's own assertion loop appends. ~keep
    let assertions_start = out.len();
    for assertion in &fixture.assertions {
        // `mock.*` is not a field of the result type at all -- it resolves against the mock
        // server's request log regardless of whether the call's result is "simple" -- so this
        // gate must not swallow it before `render_assertion`'s own mock-capture interception ever
        // runs. Without this exemption, a `result_is_simple` fixture asserting `mock.*` would be
        // misclassified as `FieldSkip::NotAccessibleOnSimpleResultType` instead of resolving
        // against the mock server, the same misattribution `assertion_mock_capture`'s module doc
        // already rules out for the `is_valid_for_result` gate inside `render_assertion` itself.
        if result_is_simple
            && let Some(f) = &assertion.field
            && !f.is_empty()
            && !crate::e2e::codegen::mock_assertions::is_mock_virtual_field(f)
        {
            let _ = writeln!(
                out,
                "  // skipped: {}",
                FieldSkip::NotAccessibleOnSimpleResultType.message(f)
            );
            continue;
        }
        render_assertion(out, assertion, "r", field_resolver, result_is_array, &pkg_module);
    }
    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&out[assertions_start..], "gleam", &fixture.id);
    crate::e2e::codegen::fail_on_unavailable_field_markers(
        &out[assertions_start..],
        "gleam",
        &fixture.id,
        &fixture.assertions,
    );

    let _ = writeln!(out, "}}");
}
