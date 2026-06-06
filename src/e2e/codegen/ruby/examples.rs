//! Ruby e2e example rendering.

use std::collections::{HashMap, HashSet};

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{ruby_string_literal, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};

use super::args::build_args_and_setup;
use super::assertions::render_assertion;
use super::spec_file::has_usable_assertion;
use super::values::json_to_ruby;
use super::visitor::build_ruby_visitor;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_chat_stream_example(
    fixture: &Fixture,
    function_name: &str,
    call_receiver: &str,
    module_name: &str,
    args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
    enum_fields: &HashMap<String, String>,
    e2e_config: &E2eConfig,
    client_factory: Option<&str>,
    extra_args: &[String],
    adapter_request_type: Option<&str>,
    streaming_item_type: Option<&str>,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.clone();
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let fixture_id = fixture.id.clone();

    let (mut setup_lines, args_str, teardown_lines) = build_args_and_setup(
        &fixture.input,
        args,
        call_receiver,
        module_name,
        options_type,
        enum_fields,
        false,
        fixture,
        adapter_request_type,
        config,
        type_defs,
    );

    let mut final_args = args_str;
    if !extra_args.is_empty() {
        let extra_str = extra_args.join(", ");
        if final_args.is_empty() {
            final_args = extra_str;
        } else {
            final_args = format!("{final_args}, {extra_str}");
        }
    }

    let mut out = String::new();
    let description_literal = ruby_string_literal(&format!("{test_name}: {description}"));
    out.push_str(&format!("  it {description_literal} do\n"));

    // Client construction.
    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    if let Some(cf) = client_factory {
        if has_mock && let Some(key_var) = api_key_var {
            let mock_url_expr = format!("\"#{{ENV['MOCK_SERVER_URL']}}/fixtures/{fixture_id}\"");
            out.push_str(&format!("    api_key = ENV['{key_var}']\n"));
            out.push_str("    if api_key && !api_key.empty?\n");
            out.push_str(&format!(
                "      warn \"{test_name}: using real API ({key_var} is set)\"\n"
            ));
            out.push_str(&format!("      client = {call_receiver}.{cf}(api_key)\n"));
            out.push_str("    else\n");
            out.push_str(&format!(
                "      warn \"{test_name}: using mock server ({key_var} not set)\"\n"
            ));
            out.push_str(&format!("      mock_url = {mock_url_expr}\n"));
            out.push_str(&format!("      client = {call_receiver}.{cf}('test-key', mock_url)\n"));
            out.push_str("    end\n");
        } else if has_mock {
            let base_url_expr = if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                format!("(ENV.fetch('{env_key}', nil) || ENV.fetch('MOCK_SERVER_URL') + '/fixtures/{fixture_id}')")
            } else {
                format!("ENV.fetch('MOCK_SERVER_URL') + '/fixtures/{fixture_id}'")
            };
            out.push_str(&format!(
                "    client = {call_receiver}.{cf}('test-key', {base_url_expr})\n"
            ));
        } else if let Some(key_var) = api_key_var {
            out.push_str(&format!("    api_key = ENV['{key_var}']\n"));
            out.push_str(&format!("    skip '{key_var} not set' unless api_key\n"));
            out.push_str(&format!("    client = {call_receiver}.{cf}(api_key)\n"));
        } else {
            out.push_str(&format!("    client = {call_receiver}.{cf}('test-key')\n"));
        }
    }

    // Visitor (rare for streaming, but support it for parity).
    if let Some(visitor_spec) = &fixture.visitor {
        let _ = build_ruby_visitor(&mut setup_lines, visitor_spec);
    }
    for line in &setup_lines {
        out.push_str(&format!("    {line}\n"));
    }

    let call_expr = if client_factory.is_some() {
        format!("client.{function_name}({final_args})")
    } else {
        format!("{call_receiver}.{function_name}({final_args})")
    };

    if expects_error {
        out.push_str(&format!("    expect {{ {call_expr} {{ |_chunk| }} }}.to raise_error\n"));
        out.push_str("  end\n");
        return out;
    }

    // Build aggregators inside a block so the iterator drives the stream synchronously.
    out.push_str("    chunks = []\n");
    out.push_str("    stream_complete = false\n");
    out.push_str(&format!("    {call_expr} do |chunk|\n"));
    out.push_str("      chunks << chunk\n");
    out.push_str("    end\n");
    out.push_str("    stream_complete = true\n");

    // Render assertions on the local aggregator vars.
    for assertion in &fixture.assertions {
        emit_chat_stream_assertion(&mut out, assertion, e2e_config, streaming_item_type);
    }

    // Always assert that the stream completed cleanly so non-empty test bodies
    // are guaranteed by RSpec's at-least-one-expectation requirement.
    if !fixture
        .assertions
        .iter()
        .any(|a| a.field.as_deref() == Some("stream_complete"))
    {
        out.push_str("    expect(stream_complete).to be(true)\n");
    }

    // Trait-bridge teardown (e.g. unregister test backend) so RSpec's
    // shared-process registry state is restored between tests.
    for line in &teardown_lines {
        out.push_str(&format!("    {line}\n"));
    }

    out.push_str("  end\n");
    out
}

/// Map a streaming fixture assertion to an `expect` call on the local aggregator
/// variable produced by [`render_chat_stream_example`]. Pseudo-fields like
/// `chunks` / `stream_content` / `stream_complete` resolve to the in-block locals,
/// not response accessors.
pub(super) fn emit_chat_stream_assertion(
    out: &mut String,
    assertion: &Assertion,
    _e2e_config: &E2eConfig,
    streaming_item_type: Option<&str>,
) {
    let atype = assertion.assertion_type.as_str();
    if atype == "not_error" || atype == "error" {
        return;
    }
    let field = assertion.field.as_deref().unwrap_or("");

    enum Kind {
        Chunks,
        Bool,
        Str,
        IntTokens,
        Json,
        Unsupported,
    }

    // Use StreamingFieldResolver to compute field expressions from chunks.
    let expr_opt = crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor_with_streaming_context(
        field,
        "ruby",
        "chunks",
        None,
        streaming_item_type,
    );

    let (expr, kind) = match (field, expr_opt) {
        ("chunks", Some(expr)) => (expr, Kind::Chunks),
        ("chunks.length", Some(expr)) => (expr, Kind::Chunks),
        ("stream_content", Some(expr)) => (expr, Kind::Str),
        ("finish_reason", Some(expr)) => (expr, Kind::Str),
        ("tool_calls", Some(expr)) => (expr, Kind::Json),
        ("tool_calls[0].function.name", Some(expr)) => (expr, Kind::Str),
        ("usage.total_tokens", Some(expr)) => (expr, Kind::IntTokens),
        ("stream_complete", None) => ("stream_complete".to_string(), Kind::Bool),
        ("no_chunks_after_done", None) => ("stream_complete".to_string(), Kind::Bool),
        _ => ("".to_string(), Kind::Unsupported),
    };

    if matches!(kind, Kind::Unsupported) {
        out.push_str(&format!(
            "    # skipped: streaming assertion on unsupported field '{field}'\n"
        ));
        return;
    }

    match (atype, &kind) {
        ("count_min", Kind::Chunks) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                out.push_str(&format!("    expect({expr}.length).to be >= {n}\n"));
            }
        }
        ("count_equals", Kind::Chunks) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                out.push_str(&format!("    expect({expr}.length).to eq({n})\n"));
            }
        }
        ("equals", Kind::Str) => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                // Mirror Python's `expr.strip() == expected.strip()` pattern: converters
                // commonly emit a trailing newline that fixture authors don't write into the
                // expected string, so strip both sides for the equality check.
                out.push_str(&format!("    expect({expr}.to_s.strip).to eq({rb_val}.strip)\n"));
            }
        }
        ("contains", Kind::Str) => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                out.push_str(&format!("    expect({expr}.to_s).to include({rb_val})\n"));
            }
        }
        ("not_empty", Kind::Str) => {
            out.push_str(&format!("    expect({expr}.to_s).not_to be_empty\n"));
        }
        ("not_empty", Kind::Json) => {
            out.push_str(&format!("    expect({expr}).not_to be_nil\n"));
        }
        ("is_empty", Kind::Str) => {
            out.push_str(&format!("    expect({expr}.to_s).to be_empty\n"));
        }
        ("is_true", Kind::Bool) => {
            out.push_str(&format!("    expect({expr}).to be(true)\n"));
        }
        ("is_false", Kind::Bool) => {
            out.push_str(&format!("    expect({expr}).to be(false)\n"));
        }
        ("greater_than_or_equal", Kind::IntTokens) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                out.push_str(&format!("    expect({expr}).to be >= {n}\n"));
            }
        }
        ("equals", Kind::IntTokens) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                out.push_str(&format!("    expect({expr}).to eq({n})\n"));
            }
        }
        _ => {
            out.push_str(&format!(
                "    # skipped: streaming assertion '{atype}' on field '{field}' not supported\n"
            ));
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_example(
    fixture: &Fixture,
    function_name: &str,
    call_receiver: &str,
    module_name: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    field_resolver: &FieldResolver,
    options_type: Option<&str>,
    enum_fields: &HashMap<String, String>,
    fields_enum: &HashSet<String>,
    result_is_simple: bool,
    returns_void: bool,
    e2e_config: &E2eConfig,
    client_factory: Option<&str>,
    extra_args: &[String],
    adapter_request_type: Option<&str>,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    let test_name = sanitize_ident(&fixture.id);
    let description_literal = ruby_string_literal(&format!("{test_name}: {}", fixture.description));
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let fixture_id = fixture.id.clone();

    let (mut setup_lines, args_str, teardown_lines) = build_args_and_setup(
        &fixture.input,
        args,
        call_receiver,
        module_name,
        options_type,
        enum_fields,
        result_is_simple,
        fixture,
        adapter_request_type,
        config,
        type_defs,
    );

    // Build visitor if present and add to setup
    let mut visitor_arg = String::new();
    if let Some(visitor_spec) = &fixture.visitor {
        visitor_arg = build_ruby_visitor(&mut setup_lines, visitor_spec);
    }

    let mut final_args = if visitor_arg.is_empty() {
        args_str
    } else if args_str.is_empty() {
        visitor_arg
    } else {
        format!("{args_str}, {visitor_arg}")
    };

    // Append per-fixture extra_args (e.g. trailing `nil` for `list_files(purpose)`).
    if !extra_args.is_empty() {
        let extra_str = extra_args.join(", ");
        if final_args.is_empty() {
            final_args = extra_str;
        } else {
            final_args = format!("{final_args}, {extra_str}");
        }
    }

    // When client_factory is configured, create a client instance and call methods on it.
    let call_expr = if client_factory.is_some() {
        format!("client.{function_name}({final_args})")
    } else {
        format!("{call_receiver}.{function_name}({final_args})")
    };

    // Check if any non-error assertion actually uses the result variable.
    let has_usable = has_usable_assertion(fixture, field_resolver, result_is_simple);

    // Render all assertions upfront into a string
    let mut assertions_rendered = String::new();
    for assertion in &fixture.assertions {
        render_assertion(
            &mut assertions_rendered,
            assertion,
            result_var,
            field_resolver,
            result_is_simple,
            e2e_config,
            fields_enum,
            enum_fields,
        );
    }

    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    let has_mock_and_key = has_mock && api_key_var.is_some();
    let has_not_error = fixture.assertions.iter().any(|a| a.assertion_type == "not_error");
    let is_only_not_error = has_not_error && !has_usable && !expects_error;

    // Detect clear operations and emit post-clear list assertion
    let is_clear_op = function_name.ends_with("_clear");
    let post_clear_list_call = if is_clear_op {
        let list_fn = function_name.replace("_clear", "_list");
        format!("{}.{}()", call_receiver, list_fn)
    } else {
        String::new()
    };

    crate::e2e::template_env::render(
        "ruby/test_function.jinja",
        minijinja::context! {
            test_name => test_name,
            description => description_literal,
            expects_error => expects_error,
            setup_lines => setup_lines,
            call_expr => call_expr,
            result_var => result_var,
            assertions_rendered => assertions_rendered,
            has_usable => has_usable,
            returns_void => returns_void,
            client_factory => client_factory,
            fixture_id => fixture_id,
            call_receiver => call_receiver,
            has_mock => has_mock,
            api_key_var => api_key_var,
            has_mock_and_key => has_mock_and_key,
            is_only_not_error => is_only_not_error,
            is_clear_op => is_clear_op,
            post_clear_list_call => post_clear_list_call,
            teardown_lines => teardown_lines,
        },
    )
}
