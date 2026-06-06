//! Gleam fixture test-case renderer.

use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_ident;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use heck::ToSnakeCase;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;

use super::args::build_args_and_setup;
use super::assertions::render_assertion;

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
) {
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        e2e_config.effective_fields_method_calls(call_config),
    );
    let field_resolver = &call_field_resolver;
    let enum_fields = e2e_config.effective_fields_enum(call_config);
    let lang = "gleam";
    let call_overrides = call_config.overrides.get(lang);
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
    let result_var = &call_config.result_var;
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
    );

    let _ = writeln!(out, "// {description}");
    let _ = writeln!(out, "pub fn {test_name}_test() {{");

    let Some((setup_lines, args_str)) = build_result else {
        let _ = writeln!(
            out,
            "  // skipped: json_object arg requires typed record construction not yet supported in Gleam e2e"
        );
        let _ = writeln!(out, "  Nil");
        let _ = writeln!(out, "}}");
        return;
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
            let _ = writeln!(out, "  {module_path}.{function_name}({full_args}) |> should.be_error()");
            let _ = writeln!(out, "}}");
            return;
        }
        let _ = writeln!(out, "  let {result_var} = {module_path}.{function_name}({full_args})");
        None
    } else {
        if expects_error {
            let _ = writeln!(out, "  {module_path}.{function_name}({args_str}) |> should.be_error()");
            let _ = writeln!(out, "}}");
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

    let mut effective_enum_fields: HashSet<String> = enum_fields.clone();
    if let Some(o) = call_overrides {
        for k in o.enum_fields.keys() {
            effective_enum_fields.insert(k.clone());
        }
        for k in o.assert_enum_fields.keys() {
            effective_enum_fields.insert(k.clone());
        }
    }

    for assertion in &fixture.assertions {
        if result_is_simple {
            if let Some(f) = &assertion.field {
                if !f.is_empty() {
                    let _ = writeln!(out, "  // skipped: field '{f}' not accessible on simple result type");
                    continue;
                }
            }
        }
        render_assertion(
            out,
            assertion,
            "r",
            field_resolver,
            &effective_enum_fields,
            result_is_array,
            &pkg_module,
        );
    }

    let _ = writeln!(out, "}}");
}
