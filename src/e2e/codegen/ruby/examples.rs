//! Ruby e2e example rendering.

use std::collections::{HashMap, HashSet};

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{ruby_regex_literal, ruby_string_literal, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;

use crate::e2e::codegen::inert_example::{self, InertCause, InertExample};

use super::args::build_args_and_setup;
use super::assertions::render_assertion;
use super::visitor::build_ruby_visitor;

/// Build the RSpec `raise_error(...)` matcher expression for an `error`-asserting test.
///
/// ~keep With no declared value this returns the original bare `raise_error(RuntimeError)`
/// unchanged, byte-for-byte, for fixtures predating this check. When a value is declared,
/// checking `error.message` OR `error.class.name` against the same regex mirrors the
/// message-or-type disjunction other backends use (see `declared_error_value`'s doc
/// comment): config-validation fixtures name text that only appears in the message,
/// API-error fixtures name a type prefix that only appears in the class name. Which of those
/// two conventions applies, and whether Ruby can ever satisfy the second, is decided once by
/// `declared_error_variant::classify` — see its doc for why Ruby lands on "not yet" today
/// (every fallible call throws a fixed `RuntimeError`, never a per-variant class).
///
/// ~keep When the declared value names a real variant Ruby cannot substantiate, this still
/// returns a single-expression matcher (spliced into `}.to {{ raise_error_clause }}` with
/// nothing else on that template line) — `raise_error(RuntimeError)`, the same honest "the call
/// must fail" check the undeclared case renders — with the registered skip appended as a
/// trailing `#`-comment on that same line, so it is visible in the generated file AND counted on
/// the shared ledger.
pub(super) fn render_raise_error_clause(fixture: &Fixture, errors: &[crate::core::ir::ErrorDef]) -> String {
    use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, skip_line};
    match classify("ruby", fixture, errors) {
        DeclaredErrorAssertion::Undeclared => "raise_error(RuntimeError)".to_string(),
        DeclaredErrorAssertion::Assert(declared) => {
            let regex = ruby_regex_literal(declared);
            format!(
                "raise_error(RuntimeError) {{ |error|\n      expect(error.message =~ {regex} || error.class.name =~ {regex}).to be_truthy\n    }}"
            )
        }
        DeclaredErrorAssertion::Unsubstantiable(variant) => {
            format!(
                "raise_error(RuntimeError) {}",
                skip_line("", "#", variant, &fixture.id, "ruby")
            )
        }
    }
}

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

    let (mut setup_lines, args_str, mut teardown_lines) = build_args_and_setup(
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

    // Emit setup calls (e.g., register_reranker_backend before calling rerank).
    for setup_call in &fixture.setup {
        let setup_call_config = e2e_config.resolve_call_for_fixture(
            Some(&setup_call.call),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &setup_call.input,
        );
        // Setup calls are overwhelmingly trait-bridge registry operations
        // (`register_reranker_backend`), which is precisely the shape that carries an empty
        // base `function` and names itself only per language. Reading the base directly here
        // rendered `client.()` — invalid Ruby. Nothing has been pushed yet at this point, so
        // a call that names no Ruby symbol drops out whole rather than emitting a fragment. ~keep
        let Some(setup_fn) = setup_call_config.effective_function("ruby") else {
            continue;
        };
        let setup_args = &setup_call_config.args;
        let (setup_setup_lines, setup_args_str, setup_teardown_lines) = build_args_and_setup(
            &setup_call.input,
            setup_args,
            call_receiver,
            module_name,
            None,
            enum_fields,
            false,
            fixture,
            None,
            config,
            type_defs,
        );

        for line in setup_setup_lines {
            setup_lines.push(line);
        }

        let setup_call_expr = if setup_args_str.is_empty() {
            format!("{}.{}()", call_receiver, setup_fn)
        } else {
            format!("{}.{}({})", call_receiver, setup_fn, setup_args_str)
        };
        setup_lines.push(setup_call_expr);

        for line in setup_teardown_lines {
            teardown_lines.push(line);
        }
    }

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

    // `stream_complete` is defined -- for every backend, see `streaming_assertions/model.rs`'s
    // field table -- as "the last collected chunk carries a terminal finish_reason". Only a
    // chat-shaped chunk has one, and the probe for that is the same set of chat pseudo-fields
    // `csharp/streaming.rs` gates its aggregators on: reaching for `chunk.choices` on any other
    // item type raises NoMethodError at spec runtime.
    //
    // This local used to be assigned `false` and then `true` unconditionally, one line above
    // `expect(stream_complete).to be(true)` -- a green check that could not fail whatever the
    // stream did. Deriving it from the collected chunks (via the resolver's own ruby accessor, so
    // ruby means by the field exactly what the other backends mean) is what makes it falsifiable.
    // ~keep
    let is_chat_stream = fixture.assertions.iter().any(|a| {
        matches!(
            a.field.as_deref(),
            Some(
                "stream_content"
                    | "finish_reason"
                    | "tool_calls"
                    | "tool_calls[0].function.name"
                    | "usage.total_tokens"
            )
        )
    });
    let stream_complete_expr = if is_chat_stream {
        crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor("stream_complete", "ruby", "chunks")
    } else {
        None
    };
    // ~keep Buffered rather than pushed straight into `out`: the refusal below has to ask whether
    // the ASSERTIONS rendered anything, and `out` already carries `chunks = []`, the call block
    // and the client setup, so a check over the whole example would answer "yes" for every
    // fixture and see none of them.
    let mut assertions_body = String::new();
    for assertion in &fixture.assertions {
        super::streaming_assertion::emit_chat_stream_assertion(
            &mut assertions_body,
            assertion,
            e2e_config,
            streaming_item_type,
            stream_complete_expr.is_some(),
        );
    }

    // `stream_complete` is asserted only where the fixture itself declares that field --
    // `emit_chat_stream_assertion` above already renders it (or its skip marker) for every
    // declared assertion. A fixture that never mentions `stream_complete` gets no expectation
    // synthesised here: e.g. `empty_stream` declares `count_min chunks >= 0`, an explicit
    // statement that zero chunks is acceptable, so inventing `expect(stream_complete).to
    // be(true)` would contradict the fixture rather than check it. ~keep

    crate::e2e::codegen::fail_on_unavailable_field_markers(&assertions_body, "ruby", &fixture.id, &fixture.assertions);
    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&assertions_body, "ruby", &fixture.id);

    // ~keep A streaming example has no honest fallback subject of its own. `chunks` is assigned
    // the literal `[]` before the drive, so `expect(chunks).not_to be_nil` could never fail, and
    // asserting it is non-empty invents an expectation no fixture declared — the very move the
    // `stream_complete` rider above was pulled back from. The one exception is a fixture that
    // declared `not_error`: "the stream does not raise" IS a real check, and it is made visible by
    // wrapping the drive rather than by refusing the example and deleting it. Everything else is
    // refused outright rather than published as a green example.
    let verdict = inert_example::inert_verdict(&assertions_body, "ruby", &fixture.id, &fixture.assertions);
    let declares_not_error = fixture.assertions.iter().any(|a| a.assertion_type == "not_error");
    if let Some(refusal) = &verdict
        && !declares_not_error
    {
        inert_example::record_refusal(refusal);
        return render_refused_example(&description_literal, &assertions_body, refusal);
    }

    // Build aggregators inside a block so the iterator drives the stream synchronously.
    //
    // ~keep The drive is wrapped in `expect { .. }.not_to raise_error` whenever nothing else in
    // this example asserts anything — the streaming counterpart of `test_function.jinja`'s
    // `has_usable` fallback, and derived from the same rendered text rather than from a predicate
    // over the fixture. That covers the assertionless "just call it" streaming smoke fixture as
    // well as a `not_error`-only one: both really do check that the stream ran without raising,
    // and this is what makes that check something a runner can see and a stream can fail.
    let drive_is_the_only_check = !inert_example::has_executable_line(&assertions_body);
    out.push_str("    chunks = []\n");
    if drive_is_the_only_check {
        out.push_str("    expect {\n");
        out.push_str(&format!("      {call_expr} do |chunk|\n"));
        out.push_str("        chunks << chunk\n");
        out.push_str("      end\n");
        out.push_str("    }.not_to raise_error\n");
    } else {
        out.push_str(&format!("    {call_expr} do |chunk|\n"));
        out.push_str("      chunks << chunk\n");
        out.push_str("    end\n");
    }
    if let Some(expr) = &stream_complete_expr {
        out.push_str(&format!("    stream_complete = {expr}\n"));
    }
    out.push_str(&assertions_body);

    // Trait-bridge teardown (e.g. unregister test backend) so RSpec's
    // shared-process registry state is restored between tests.
    for line in &teardown_lines {
        out.push_str(&format!("    {line}\n"));
    }

    out.push_str("  end\n");
    out
}

#[cfg(test)]
mod stream_complete_declaration_gate_tests {
    use super::render_chat_stream_example;
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::{Assertion, Fixture};

    fn render(id: &str, assertions: Vec<Assertion>) -> String {
        let fixture = Fixture {
            id: id.to_string(),
            description: "test".to_string(),
            assertions,
            ..Fixture::default()
        };
        render_chat_stream_example(
            &fixture,
            "chat_stream",
            "client",
            "SampleCrate",
            &[],
            None,
            &std::collections::HashMap::new(),
            &E2eConfig::default(),
            None,
            &[],
            None,
            None,
            &ResolvedCrateConfig::default(),
            &[],
        )
    }

    /// Regression test for the fabricated-completion defect: a fixture that never declares
    /// `stream_complete` (here, `empty_stream`'s real shape -- `count_min chunks >= 0` plus
    /// `equals stream_content == ""`, an explicit statement that zero chunks is acceptable)
    /// must not have `expect(stream_complete).to be(true)` invented on its behalf. That
    /// expectation would contradict rather than check a fixture like this one. ~keep
    #[test]
    fn a_fixture_that_never_declares_stream_complete_gets_no_invented_expectation() {
        let out = render(
            "empty_stream",
            vec![
                Assertion {
                    assertion_type: "count_min".to_string(),
                    field: Some("chunks".to_string()),
                    value: Some(serde_json::json!(0)),
                    ..Default::default()
                },
                Assertion {
                    assertion_type: "equals".to_string(),
                    field: Some("stream_content".to_string()),
                    value: Some(serde_json::json!("")),
                    ..Default::default()
                },
            ],
        );

        assert!(
            !out.contains("expect(stream_complete)"),
            "no expectation may be invented for a field this fixture never declared. got:\n{out}"
        );
    }

    /// The other half: a fixture that DOES declare `stream_complete` (the two real liter-llm
    /// fixtures `local_stream_ollama` and `stream_done_signal` shape) must still get a real,
    /// falsifiable expectation -- the fix must not regress the declared case into silence.
    #[test]
    fn a_fixture_that_declares_stream_complete_still_gets_a_real_expectation() {
        let out = render(
            "stream_done_signal",
            vec![
                Assertion {
                    assertion_type: "equals".to_string(),
                    field: Some("stream_content".to_string()),
                    value: Some(serde_json::json!("done")),
                    ..Default::default()
                },
                Assertion {
                    assertion_type: "is_true".to_string(),
                    field: Some("stream_complete".to_string()),
                    ..Default::default()
                },
            ],
        );

        assert_eq!(
            out.matches("expect(stream_complete).to be(true)").count(),
            1,
            "a declared `stream_complete` assertion must render exactly once. got:\n{out}"
        );
    }
}

/// The example emitted in place of one whose assertions all funnelled into skip markers.
///
/// ~keep The rendered markers are carried into the refusal rather than dropped: they are the only
/// record IN THE GENERATED FILE of what the fixture asked for and why it could not run, and the
/// point of this change is to stop publishing a green test, not to restore the silence the markers
/// were added to break.
///
/// Which refusal is emitted follows who can fix it. An unresolved field path is the consumer's to
/// repair — under the default strict setting the run has already failed, so this shape only ever
/// reaches a generated file on a deliberately disarmed run, and that run must still not go green:
/// it gets a real expectation that always fails and names the fixture. Everything else is alef's
/// generator debt or a language limit that no consumer edit clears; failing their suite for it
/// would only force a blanket opt-out, so it gets RSpec's own `skip`, which reports the example as
/// pending and never as a pass. `skip` also keeps the example present, so the enclosing `describe`
/// does not go empty.
fn render_refused_example(description_literal: &str, markers: &str, refusal: &InertExample) -> String {
    let mut out = format!("  it {description_literal} do\n");
    for line in markers.lines() {
        if line.trim().is_empty() {
            continue;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    let reason = ruby_string_literal(&refusal.reason());
    match refusal.cause {
        InertCause::UnresolvedFieldPath => {
            out.push_str(&format!("    unresolved_assertion = {reason}\n"));
            out.push_str("    expect(unresolved_assertion).to be_nil\n");
        }
        InertCause::AwaitedOrLimited | InertCause::RenderedNothing => {
            out.push_str(&format!("    skip {reason}\n"));
        }
    }
    out.push_str("  end\n");
    out
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
    errors: &[crate::core::ir::ErrorDef],
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
    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&assertions_rendered, "ruby", &fixture.id);
    crate::e2e::codegen::fail_on_unavailable_field_markers(
        &assertions_rendered,
        "ruby",
        &fixture.id,
        &fixture.assertions,
    );

    // Detect clear operations: `test_function.jinja` emits its own `expect(list_result).to
    // be_empty` for them, so a clear-op example asserts something whatever the assertion body
    // rendered and must never be refused. Computed here rather than further down so the refusal
    // below can consult it. ~keep
    let is_clear_op = function_name.ends_with("_clear");

    // ~keep `has_usable` used to be `has_usable_assertion(..)`, a PRE-render predicate over
    // `fixture.assertions`. That answers a different question than the template asks: the
    // template needs to know whether the body it is about to splice in asserts anything, and the
    // predicate guessed that from the fixture before any renderer had its say. The two disagree
    // wherever ruby's own renderer drops an assertion the predicate accepted — the serialized-enum
    // accessor, the nested array-wildcard refusal, a `result_is_simple` field no arm can express —
    // and on exactly that disagreement the `expect(result).not_to be_nil` fallback did not fire
    // and the example was published asserting nothing. Deriving it from the rendered text is what
    // removes the drift; typescript and python already made the same move for this defect.
    let verdict = if expects_error || is_clear_op {
        None
    } else {
        inert_example::inert_verdict(&assertions_rendered, "ruby", &fixture.id, &fixture.assertions)
    };
    // ~keep Only an unresolved field path is refused outright here. The other inert causes still
    // have an honest, FAILABLE expectation available to them in `test_function.jinja` —
    // `expect(result).not_to be_nil` when a result is bound, `expect { .. }.not_to raise_error`
    // when the call returns void — and refusing those would delete the "the call worked" coverage
    // they do carry. An unresolved path is different: the fixture named a check that a config or
    // fixture edit would make run, so letting a generic fallback pass in its place is the green
    // test this change exists to stop. Under the default strict setting the run has already failed
    // on it; this branch is what a deliberately disarmed run gets instead of silence.
    if let Some(refusal) = &verdict
        && refusal.cause == InertCause::UnresolvedFieldPath
    {
        inert_example::record_refusal(refusal);
        return render_refused_example(&description_literal, &assertions_rendered, refusal);
    }
    // ~keep Derived from the rendered text, NOT from `refusal.is_none()`: a fixture that declares
    // no assertions at all is never refused (the deliberate "just call it" smoke contract) and
    // must still take the template's `expect(result).not_to be_nil` branch, exactly as it did
    // before this change.
    let has_usable = inert_example::has_executable_line(&assertions_rendered);

    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    let has_mock_and_key = has_mock && api_key_var.is_some();

    let raise_error_clause = render_raise_error_clause(fixture, errors);

    // Emit the post-clear list assertion for clear operations.
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
            raise_error_clause => raise_error_clause,
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
            is_clear_op => is_clear_op,
            post_clear_list_call => post_clear_list_call,
            teardown_lines => teardown_lines,
        },
    )
}

#[cfg(test)]
mod dropped_field_marker_tests {
    use super::render_example;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::field_access::FieldResolver;
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

    /// Regression test for alef task #81: Ruby's "skipped: field not available" comment
    /// must carry the exact marker text the shared `fail_on_unavailable_field_markers`
    /// mechanism (src/e2e/codegen/mod.rs) matches on, so arming
    /// `ALEF_E2E_STRICT_FIELD_AVAILABILITY` turns a dropped field assertion into a
    /// generation-time failure. The arming behaviour itself is proven in `mod.rs`'s
    /// `unavailable_field_marker_tests`; this test only pins the marker text Ruby emits
    /// through the real per-fixture rendering entry point. ~keep
    #[test]
    fn dropped_field_assertion_carries_the_marker_that_arms_the_strict_mode() {
        let fixture = make_fixture("process_smoke", "nonexistent_field");
        let field_resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::from(["content".to_string()]),
            &HashSet::new(),
            &HashSet::new(),
        );
        let config = crate::core::config::ResolvedCrateConfig::default();
        let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();

        let out = render_example(
            &fixture,
            "process",
            "SampleCrate",
            "SampleCrate",
            "result",
            &[],
            &field_resolver,
            None,
            &HashMap::new(),
            &HashSet::new(),
            false,
            false,
            &E2eConfig::default(),
            None,
            &[],
            None,
            &config,
            &type_defs,
            &[],
        );

        assert!(
            out.contains("field 'nonexistent_field' not available on result type"),
            "got:\n{out}"
        );
    }
}

#[cfg(test)]
mod raise_error_clause_tests {
    use super::render_raise_error_clause;
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

    fn coded_error_def(variant_name: &str) -> ErrorDef {
        ErrorDef {
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: variant_name.to_string(),
                error_code: Some(100),
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    #[test]
    fn no_declared_value_is_byte_identical_to_bare_raise_error() {
        let fixture = Fixture {
            id: "no_error".to_string(),
            ..Fixture::default()
        };
        assert_eq!(render_raise_error_clause(&fixture, &[]), "raise_error(RuntimeError)");
    }

    /// With no `errors` IR supplied, a value cannot be recognised as a known variant name, so it
    /// renders exactly like a message-style value always did before this fix.
    #[test]
    fn declared_value_adds_message_or_class_name_check() {
        let fixture = fixture_with_declared_error("BadRequest");
        let clause = render_raise_error_clause(&fixture, &[]);
        assert_eq!(
            clause,
            "raise_error(RuntimeError) { |error|\n      expect(error.message =~ /BadRequest/ || error.class.name =~ /BadRequest/).to be_truthy\n    }"
        );
    }

    #[test]
    fn declared_value_with_regex_metacharacters_is_escaped() {
        let fixture = fixture_with_declared_error("field.name[0]");
        let clause = render_raise_error_clause(&fixture, &[]);
        assert!(
            clause.contains("/field\\.name\\[0\\]/"),
            "expected escaped regex literal, got: {clause}"
        );
    }

    /// The defect this fix closes: a declared value that names a real `ErrorVariant` — every
    /// Ruby fallible call throws a fixed `RuntimeError`, never a per-variant class — must render
    /// the registered skip, not a message-or-class-name comparison that can never pass.
    #[test]
    fn declared_value_naming_a_known_variant_falls_back_to_bare_raise_error_with_a_trailing_skip_comment() {
        let fixture = fixture_with_declared_error("Authentication");
        let errors = vec![coded_error_def("Authentication")];
        let clause = render_raise_error_clause(&fixture, &errors);
        assert_eq!(
            clause,
            "raise_error(RuntimeError) # skipped: declared error variant 'Authentication' not yet preserved as a \
             distinct identity by this backend's generator"
        );
    }
}

/// The refusal path: an example whose assertions all funnelled into skip markers must never be
/// published as a passing spec, and an example that still asserts something must be published
/// completely unchanged.
///
/// ~keep Every case here asserts the POSITIVE emission before any absence assertion. An absence
/// check against a renderer that emitted nothing at all would pass for the wrong reason, and the
/// regression this fix can cause — a refusal so broad it deletes working coverage — is exactly the
/// one an absence-only test cannot see.
#[cfg(test)]
mod inert_example_refusal_tests {
    use super::{render_chat_stream_example, render_example};
    use crate::e2e::codegen::inert_example::take_inert_examples;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::{Assertion, Fixture};
    use std::collections::{HashMap, HashSet};

    fn assertion(assertion_type: &str, field: &str, value: serde_json::Value) -> Assertion {
        Assertion {
            assertion_type: assertion_type.to_string(),
            field: Some(field.to_string()),
            value: Some(value),
            ..Default::default()
        }
    }

    fn fixture_with(id: &str, assertions: Vec<Assertion>) -> Fixture {
        Fixture {
            id: id.to_string(),
            description: "test".to_string(),
            assertions,
            ..Default::default()
        }
    }

    fn resolver_knowing(fields: &[&str]) -> FieldResolver {
        let result_fields: HashSet<String> = fields.iter().map(|field| (*field).to_string()).collect();
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    fn render_stream(fixture: &Fixture) -> String {
        let config = crate::core::config::ResolvedCrateConfig::default();
        let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
        render_chat_stream_example(
            fixture,
            "chat_stream",
            "SampleCrate",
            "SampleCrate",
            &[],
            None,
            &HashMap::new(),
            &E2eConfig::default(),
            None,
            &[],
            None,
            None,
            &config,
            &type_defs,
        )
    }

    fn render_plain(fixture: &Fixture, field_resolver: &FieldResolver, returns_void: bool) -> String {
        let config = crate::core::config::ResolvedCrateConfig::default();
        let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
        render_example(
            fixture,
            "process",
            "SampleCrate",
            "SampleCrate",
            "result",
            &[],
            field_resolver,
            None,
            &HashMap::new(),
            &HashSet::new(),
            false,
            returns_void,
            &E2eConfig::default(),
            None,
            &[],
            None,
            &config,
            &type_defs,
            &[],
        )
    }

    /// CONTROL, asserted first: one renderable assertion is enough to publish the example, and the
    /// skip markers beside it must survive. A refusal that fired here would delete coverage that
    /// works today and restore the silence the markers were added to break.
    #[test]
    fn a_stream_with_one_renderable_assertion_is_published_with_its_markers_intact() {
        let _ = take_inert_examples();
        let fixture = fixture_with(
            "stream_control",
            vec![
                assertion("count_min", "chunks", serde_json::json!(1)),
                assertion("is_true", "stream.has_page_event", serde_json::json!(true)),
            ],
        );

        let out = render_stream(&fixture);

        assert!(
            out.contains("expect(chunks.length).to be >= 1"),
            "the renderable assertion must still be emitted, got:\n{out}"
        );
        assert!(
            out.contains("streaming assertion on unsupported field 'stream.has_page_event'"),
            "the skip marker beside it must survive, got:\n{out}"
        );
        assert!(
            out.contains("chunks << chunk"),
            "the stream must still be driven, got:\n{out}"
        );
        assert!(
            !out.contains("    skip "),
            "a live example must not be refused, got:\n{out}"
        );
        assert!(
            take_inert_examples().is_empty(),
            "nothing may be recorded for a live example"
        );
    }

    /// The blocker: every declared assertion funnels into a `StreamingAssertionOnUnsupportedField`
    /// marker, so the example ran a real stream and asserted nothing. It must come out as a
    /// pending example, never as a passing one — and the markers must come with it.
    #[test]
    fn a_stream_whose_every_assertion_skips_is_refused_as_a_pending_example() {
        let _ = take_inert_examples();
        let fixture = fixture_with(
            "stream_all_skipped",
            vec![
                assertion("is_true", "stream.has_page_event", serde_json::json!(true)),
                assertion("is_true", "stream.has_complete_event", serde_json::json!(true)),
            ],
        );

        let out = render_stream(&fixture);

        assert!(
            out.contains("streaming assertion on unsupported field 'stream.has_page_event'")
                && out.contains("streaming assertion on unsupported field 'stream.has_complete_event'"),
            "the markers must be carried into the refusal, got:\n{out}"
        );
        assert!(
            out.contains("    skip 'alef rendered no runnable expectation for fixture `stream_all_skipped`"),
            "the refusal must be RSpec's own pending marker, naming why, got:\n{out}"
        );
        assert!(
            !out.contains("expect("),
            "a refused example must not carry an expectation that passes, got:\n{out}"
        );
        assert!(
            !out.contains("chunks << chunk"),
            "a refused example must not run the call it cannot check, got:\n{out}"
        );
        let refusals = take_inert_examples();
        assert_eq!(
            refusals.len(),
            1,
            "the refusal must be recorded once for the run summary"
        );
        assert_eq!(refusals[0].fixture_id, "stream_all_skipped");
    }

    /// CONTROL for the non-streaming path, asserted before the refusal case below: a resolvable
    /// field still renders its `expect`, and the template's own fallback must NOT be appended on
    /// top of it.
    #[test]
    fn a_resolvable_field_assertion_is_published_unchanged() {
        let _ = take_inert_examples();
        let fixture = fixture_with(
            "plain_control",
            vec![assertion("equals", "content", serde_json::json!("hello"))],
        );

        let out = render_plain(&fixture, &resolver_knowing(&["content"]), false);

        // Ruby coerces before comparing (`expect(result.content.to_s).to eq('hello')`), so the
        // accessor is a prefix of the emitted call, not the whole of it. Matching the closing
        // paren here pinned a spelling the renderer never produced. ~keep
        assert!(
            out.contains("expect(result.content"),
            "the renderable assertion must still be emitted, got:\n{out}"
        );
        assert!(
            !out.contains("expect(result).not_to be_nil"),
            "the vacuous fallback must not be appended beside a real assertion, got:\n{out}"
        );
        assert!(
            take_inert_examples().is_empty(),
            "nothing may be recorded for a live example"
        );
    }

    /// A field the availability oracle rejects is the consumer's to fix, so the disarmed run that
    /// still emits it gets an expectation that FAILS and names the fixture — never a `skip`, which
    /// would let a fixable authoring gap sit quietly in the pending column forever.
    #[test]
    fn an_unresolved_field_path_is_refused_with_a_failing_expectation() {
        let _ = take_inert_examples();
        let fixture = fixture_with(
            "plain_unresolved",
            vec![assertion("equals", "nonexistent_field", serde_json::json!("x"))],
        );

        let out = render_plain(&fixture, &resolver_knowing(&["content"]), false);

        assert!(
            out.contains("field 'nonexistent_field' not available on result type"),
            "the marker must be carried into the refusal, got:\n{out}"
        );
        assert!(
            out.contains("unresolved_assertion = 'alef resolved no assertion for fixture `plain_unresolved`")
                && out.contains("expect(unresolved_assertion).to be_nil"),
            "the refusal must be an expectation that fails, got:\n{out}"
        );
        assert!(
            !out.contains("    skip "),
            "a consumer-fixable gap must not be parked as pending, got:\n{out}"
        );
        assert_eq!(take_inert_examples().len(), 1);
    }

    /// A streaming fixture that declares `not_error` and nothing else renderable must keep driving
    /// the stream — refusing it would delete the one real check it carries. The drive is wrapped
    /// so that check becomes a visible expectation rather than an implicit one.
    #[test]
    fn a_stream_asserting_only_not_error_keeps_driving_the_stream() {
        let _ = take_inert_examples();
        let not_error = Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        };
        let fixture = fixture_with("stream_not_error_only", vec![not_error]);

        let out = render_stream(&fixture);

        assert!(
            out.contains("}.not_to raise_error"),
            "the drive must be wrapped in a real expectation, got:\n{out}"
        );
        assert!(
            out.contains("chunks << chunk"),
            "the stream must still be driven, got:\n{out}"
        );
        assert!(
            !out.contains("    skip "),
            "coverage that works today must not be parked as pending, got:\n{out}"
        );
        assert!(
            take_inert_examples().is_empty(),
            "an example that still asserts something is not a refusal"
        );
    }

    /// A fixture that declares NO assertions is the deliberate "just call it" smoke contract and
    /// must be published exactly as it was before this change. ~keep
    #[test]
    fn a_fixture_with_no_declared_assertions_keeps_its_smoke_test_shape() {
        let _ = take_inert_examples();
        let fixture = fixture_with("smoke_only", vec![]);

        let out = render_plain(&fixture, &resolver_knowing(&["content"]), false);

        assert!(
            out.contains("result = SampleCrate.process()"),
            "the call must still be emitted, got:\n{out}"
        );
        assert!(
            out.contains("expect(result).not_to be_nil"),
            "the pre-existing smoke fallback must be untouched, got:\n{out}"
        );
        assert!(
            take_inert_examples().is_empty(),
            "a fixture with no assertions must never be recorded as refused"
        );
    }
}
