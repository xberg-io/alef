//! Ruby e2e test generator using RSpec.
//!
//! Generates `e2e/ruby/Gemfile` and `spec/{category}_spec.rb` files from
//! JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

use crate::codegen::resolve_field;
use crate::config::E2eConfig;
use crate::escape::{ruby_string_literal, ruby_template_to_interpolation, sanitize_filename, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, CallbackAction, Fixture, FixtureGroup, ValidationErrorExpectation};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::template_versions as tv;
use anyhow::Result;
use heck::ToSnakeCase;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;
use super::client;

/// Ruby e2e code generator.
pub struct RubyCodegen;

impl E2eCodegen for RubyCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        _type_defs: &[alef_core::ir::TypeDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        // Resolve call config with overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let module_path = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        let class_name = overrides.and_then(|o| o.class.as_ref()).cloned();
        let options_type = overrides.and_then(|o| o.options_type.clone());
        let empty_enum_fields = HashMap::new();
        let enum_fields = overrides.map(|o| &o.enum_fields).unwrap_or(&empty_enum_fields);
        let result_is_simple = call.result_is_simple || overrides.is_some_and(|o| o.result_is_simple);

        // Resolve package config.
        let ruby_pkg = e2e_config.resolve_package("ruby");
        let gem_name = ruby_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| config.name.replace('-', "_"));
        let gem_path = ruby_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/ruby".to_string());
        let gem_version = ruby_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .or_else(|| config.resolved_version())
            .unwrap_or_else(|| "0.1.0".to_string());

        // Generate Gemfile.
        files.push(GeneratedFile {
            path: output_base.join("Gemfile"),
            content: render_gemfile(&gem_name, &gem_path, &gem_version, e2e_config.dep_mode),
            generated_header: false,
        });

        // Generate .rubocop.yaml for linting generated specs.
        files.push(GeneratedFile {
            path: output_base.join(".rubocop.yaml"),
            content: render_rubocop_yaml(),
            generated_header: false,
        });

        // Check if any fixture is an HTTP test (needs mock server bootstrap).
        let has_http_fixtures = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| f.needs_mock_server());

        // Check if any fixture uses file_path or bytes args (needs chdir to test_documents).
        let has_file_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
            let cc = e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.input);
            cc.args
                .iter()
                .any(|a| a.arg_type == "file_path" || a.arg_type == "bytes")
        });

        // Always generate spec/spec_helper.rb when file-based or HTTP fixtures are present.
        if has_file_fixtures || has_http_fixtures {
            files.push(GeneratedFile {
                path: output_base.join("spec").join("spec_helper.rb"),
                content: render_spec_helper(
                    has_file_fixtures,
                    has_http_fixtures,
                    &e2e_config.test_documents_relative_from(1),
                ),
                generated_header: true,
            });
        }

        // Generate spec files per category.
        let spec_base = output_base.join("spec");

        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let field_resolver_pre = FieldResolver::new(
                &e2e_config.fields,
                &e2e_config.fields_optional,
                &e2e_config.result_fields,
                &e2e_config.fields_array,
                &std::collections::HashSet::new(),
            );
            // Skip the entire file if no fixture in this category produces output.
            let has_any_output = active.iter().any(|f| {
                // HTTP tests always produce output.
                if f.is_http_test() {
                    return true;
                }
                let expects_error = f.assertions.iter().any(|a| a.assertion_type == "error");
                let has_not_error = f.assertions.iter().any(|a| a.assertion_type == "not_error");
                expects_error || has_not_error || has_usable_assertion(f, &field_resolver_pre, result_is_simple)
            });
            if !has_any_output {
                continue;
            }

            let filename = format!("{}_spec.rb", sanitize_filename(&group.category));
            let field_resolver = FieldResolver::new(
                &e2e_config.fields,
                &e2e_config.fields_optional,
                &e2e_config.result_fields,
                &e2e_config.fields_array,
                &std::collections::HashSet::new(),
            );
            let content = render_spec_file(
                &group.category,
                &active,
                &module_path,
                class_name.as_deref(),
                &gem_name,
                &field_resolver,
                options_type.as_deref(),
                enum_fields,
                result_is_simple,
                e2e_config,
                has_file_fixtures || has_http_fixtures,
            );
            files.push(GeneratedFile {
                path: spec_base.join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "ruby"
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_gemfile(
    gem_name: &str,
    gem_path: &str,
    gem_version: &str,
    dep_mode: crate::config::DependencyMode,
) -> String {
    let gem_line = match dep_mode {
        crate::config::DependencyMode::Registry => format!("gem '{gem_name}', '{gem_version}'"),
        crate::config::DependencyMode::Local => format!("gem '{gem_name}', path: '{gem_path}'"),
    };
    crate::template_env::render(
        "ruby/Gemfile.jinja",
        minijinja::context! {
            gem_line => gem_line,
            rspec => tv::gem::RSPEC_E2E,
            rubocop => tv::gem::RUBOCOP_E2E,
            rubocop_rspec => tv::gem::RUBOCOP_RSPEC_E2E,
            faraday => tv::gem::FARADAY,
        },
    )
}

fn render_spec_helper(has_file_fixtures: bool, has_http_fixtures: bool, test_documents_path: &str) -> String {
    let header = hash::header(CommentStyle::Hash);
    let mut out = header;
    out.push_str("# frozen_string_literal: true\n");

    if has_file_fixtures {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "# Change to the configured test-documents directory so that fixture file paths like"
        );
        let _ = writeln!(
            out,
            "# \"pdf/fake_memo.pdf\" resolve correctly when running rspec from e2e/ruby/."
        );
        let _ = writeln!(
            out,
            "# spec_helper.rb lives in e2e/ruby/spec/; the fixtures dir resolves three directories up."
        );
        let _ = writeln!(
            out,
            "_test_documents = File.expand_path('{test_documents_path}', __dir__)"
        );
        let _ = writeln!(out, "Dir.chdir(_test_documents) if Dir.exist?(_test_documents)");
    }

    if has_http_fixtures {
        out.push_str(
            r#"
require 'json'
require 'open3'

# Spawn the mock-server binary and set MOCK_SERVER_URL for all tests.
RSpec.configure do |config|
  config.before(:suite) do
    bin = File.expand_path('../../rust/target/release/mock-server', __dir__)
    fixtures_dir = File.expand_path('../../../fixtures', __dir__)
    unless File.exist?(bin)
      warn "mock-server binary not found at #{bin} — run: cargo build --manifest-path e2e/rust/Cargo.toml --bin mock-server --release"
    end
    stdin, stdout, _stderr, _wait = Open3.popen3(bin, fixtures_dir)
    # Read startup lines: MOCK_SERVER_URL= then optional MOCK_SERVERS=.
    url = nil
    8.times do
      line = stdout.readline.strip rescue break
      if line.start_with?('MOCK_SERVER_URL=')
        url = line.split('=', 2).last
        ENV['MOCK_SERVER_URL'] = url
      elsif line.start_with?('MOCK_SERVERS=')
        json_val = line.split('=', 2).last
        ENV['MOCK_SERVERS'] = json_val
        JSON.parse(json_val).each do |fid, furl|
          ENV["MOCK_SERVER_#{fid.upcase}"] = furl
        end
        break
      elsif url
        break
      end
    end
    # Drain stdout in background.
    Thread.new { stdout.read }
    # Store stdin so we can close it on teardown.
    @_mock_server_stdin = stdin
  end

  config.after(:suite) do
    @_mock_server_stdin&.close
  end
end
"#,
        );
    }

    out
}

fn render_rubocop_yaml() -> String {
    crate::template_env::render("ruby/rubocop.yml.jinja", minijinja::context! {})
}

#[allow(clippy::too_many_arguments)]
fn render_spec_file(
    category: &str,
    fixtures: &[&Fixture],
    module_path: &str,
    class_name: Option<&str>,
    gem_name: &str,
    field_resolver: &FieldResolver,
    options_type: Option<&str>,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    needs_spec_helper: bool,
) -> String {
    // Resolve client_factory from ruby override.
    let client_factory = e2e_config
        .call
        .overrides
        .get("ruby")
        .and_then(|o| o.client_factory.as_deref());

    // Build requires list
    let require_name = if module_path.is_empty() { gem_name } else { module_path };
    let mut requires = vec![require_name.replace('-', "_"), "json".to_string()];

    let has_http = fixtures.iter().any(|f| f.is_http_test());
    if needs_spec_helper || has_http {
        requires.push("spec_helper".to_string());
    }

    // Build the Ruby module/class qualifier for calls.
    let call_receiver = class_name
        .map(|s| s.to_string())
        .unwrap_or_else(|| ruby_module_name(module_path));

    // Check for array contains assertions
    let has_array_contains = fixtures.iter().any(|fixture| {
        fixture.assertions.iter().any(|a| {
            matches!(a.assertion_type.as_str(), "contains" | "contains_all" | "not_contains")
                && a.field
                    .as_deref()
                    .is_some_and(|f| !f.is_empty() && field_resolver.is_array(field_resolver.resolve(f)))
        })
    });

    // Build examples
    let mut examples = Vec::new();
    for fixture in fixtures {
        if fixture.http.is_some() {
            // HTTP example is handled separately (uses shared driver)
            let mut out = String::new();
            render_http_example(&mut out, fixture);
            examples.push(out);
        } else {
            // Resolve per-fixture call config so we can detect streaming up front.
            let fixture_call = e2e_config.resolve_call_for_fixture(fixture.call.as_deref(), &fixture.input);
            let fixture_call_overrides = fixture_call.overrides.get("ruby");
            let raw_function_name = fixture_call_overrides
                .and_then(|o| o.function.as_ref())
                .cloned()
                .unwrap_or_else(|| fixture_call.function.clone());

            let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
            let has_not_error = fixture.assertions.iter().any(|a| a.assertion_type == "not_error");
            let has_usable = has_usable_assertion(fixture, field_resolver, result_is_simple);
            let is_streaming = raw_function_name == "chat_stream";

            // Non-HTTP, non-streaming fixtures with no usable assertions stay pending.
            // A fixture whose only assertion is `not_error` is still testable — it
            // verifies the call does not raise, so route it to render_example.
            if !expects_error && !has_usable && !has_not_error && !is_streaming {
                let test_name = sanitize_ident(&fixture.id);
                let description = fixture.description.replace('\'', "\\'");
                let mut out = String::new();
                out.push_str(&format!("  it '{test_name}: {description}' do\n"));
                out.push_str("    skip 'Non-HTTP fixture cannot be tested via Net::HTTP'\n");
                out.push_str("  end\n");
                examples.push(out);
            } else {
                // Streaming methods do not take the `_async` suffix — Magnus emits
                // `chat_stream` as a block-yielding method. All other async Rust
                // methods are bound with the `_async` suffix.
                let fixture_function_name = if is_streaming {
                    raw_function_name
                } else if fixture_call.r#async && !raw_function_name.ends_with("_async") {
                    format!("{raw_function_name}_async")
                } else {
                    raw_function_name
                };
                let fixture_result_var = &fixture_call.result_var;
                let fixture_args = &fixture_call.args;
                let fixture_client_factory = fixture_call_overrides
                    .and_then(|o| o.client_factory.as_deref())
                    .or(client_factory);
                let fixture_options_type = fixture_call_overrides
                    .and_then(|o| o.options_type.as_deref())
                    .or(options_type);

                let fixture_extra_args: Vec<String> =
                    fixture_call_overrides.map(|o| o.extra_args.clone()).unwrap_or_default();
                // Use per-fixture-call result_is_simple so per-call overrides like
                // `speech` (returns bytes) take precedence over the top-level call default.
                let fixture_result_is_simple =
                    fixture_call.result_is_simple || fixture_call_overrides.is_some_and(|o| o.result_is_simple);
                // Per-call enum_fields take precedence — e.g. `[crates.e2e.calls.create_batch.overrides.ruby] enum_fields`
                // labels `status = "BatchStatus"` for the batch lifecycle, but the global
                // `[crates.e2e.call.overrides.ruby]` map only carries chat-shape entries.
                let fixture_enum_fields: &HashMap<String, String> =
                    fixture_call_overrides.map(|o| &o.enum_fields).unwrap_or(enum_fields);
                let example = if is_streaming {
                    render_chat_stream_example(
                        fixture,
                        &fixture_function_name,
                        &call_receiver,
                        fixture_args,
                        fixture_options_type,
                        fixture_enum_fields,
                        e2e_config,
                        fixture_client_factory,
                        &fixture_extra_args,
                    )
                } else {
                    render_example(
                        fixture,
                        &fixture_function_name,
                        &call_receiver,
                        fixture_result_var,
                        fixture_args,
                        field_resolver,
                        fixture_options_type,
                        fixture_enum_fields,
                        fixture_result_is_simple,
                        e2e_config,
                        fixture_client_factory,
                        &fixture_extra_args,
                    )
                };
                examples.push(example);
            }
        }
    }

    let header = hash::header(CommentStyle::Hash);
    crate::template_env::render(
        "ruby/test_file.jinja",
        minijinja::context! {
            category => category,
            requires => requires,
            has_array_contains => has_array_contains,
            has_http => has_http,
            examples => examples,
            header => header,
        },
    )
}

/// Check if a fixture has at least one assertion that will produce an executable
/// expect() call (not just a skip comment).
fn has_usable_assertion(fixture: &Fixture, field_resolver: &FieldResolver, result_is_simple: bool) -> bool {
    fixture.assertions.iter().any(|a| {
        // not_error is implicit (call succeeding), error is handled separately.
        if a.assertion_type == "not_error" || a.assertion_type == "error" {
            return false;
        }
        // Check field validity.
        if let Some(f) = &a.field {
            if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
                return false;
            }
            // When result_is_simple, skip non-content fields.
            if result_is_simple {
                let f_lower = f.to_lowercase();
                if !f.is_empty()
                    && f_lower != "content"
                    && (f_lower.starts_with("metadata")
                        || f_lower.starts_with("document")
                        || f_lower.starts_with("structure"))
                {
                    return false;
                }
            }
        }
        true
    })
}

// ---------------------------------------------------------------------------
// HTTP test rendering — shared-driver integration
// ---------------------------------------------------------------------------

/// Thin renderer that emits RSpec `describe` + `it` blocks targeting a mock server
/// via `Net::HTTP`. Satisfies [`client::TestClientRenderer`] so the shared
/// [`client::http_call::render_http_test`] driver drives the call sequence.
struct RubyTestClientRenderer;

impl client::TestClientRenderer for RubyTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "ruby"
    }

    /// Emit `describe '{fn_name}' do` + inner `it '{description}' do`.
    ///
    /// `fn_name` is the sanitised fixture id used as the describe label.
    /// When `skip_reason` is `Some`, the inner `it` block gets a `skip` call so
    /// the shared driver short-circuits before emitting any assertions.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let escaped_description = description.replace('\'', "\\'");
        let rendered = crate::template_env::render(
            "ruby/http_test.jinja",
            minijinja::context! {
                fn_name => fn_name,
                description => escaped_description,
                skip_reason => skip_reason,
            },
        );
        out.push_str(&rendered);
    }

    /// Close the inner `it` block and the outer `describe` block.
    fn render_test_close(&self, out: &mut String) {
        let rendered = crate::template_env::render("ruby/http_test_close.jinja", minijinja::context! {});
        out.push_str(&rendered);
    }

    /// Emit a `Net::HTTP` request to the mock server using the path from `ctx`.
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();
        let method_class = http_method_class(&method);

        let has_body = ctx
            .body
            .is_some_and(|b| !matches!(b, serde_json::Value::String(s) if s.is_empty()));

        let ruby_body = if has_body {
            json_to_ruby(ctx.body.unwrap())
        } else {
            String::new()
        };

        let headers: Vec<minijinja::Value> = ctx
            .headers
            .iter()
            .filter(|(k, _)| {
                // Skip Content-Type when already set from the body above.
                !(has_body && k.to_lowercase() == "content-type")
            })
            .map(|(k, v)| {
                minijinja::context! {
                    key_literal => ruby_string_literal(k),
                    value_literal => ruby_string_literal(v),
                }
            })
            .collect();

        let rendered = crate::template_env::render(
            "ruby/http_request.jinja",
            minijinja::context! {
                method_class => method_class,
                path => ctx.path,
                has_body => has_body,
                ruby_body => ruby_body,
                headers => headers,
                response_var => ctx.response_var,
            },
        );
        out.push_str(&rendered);
    }

    /// Emit `expect(response.code.to_i).to eq(status)`.
    ///
    /// Net::HTTP returns the HTTP status as a `String`; `.to_i` converts it for
    /// comparison with the integer literal from the fixture.
    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        out.push_str(&format!("      expect({response_var}.code.to_i).to eq({status})\n"));
    }

    /// Emit a header assertion using `response[header_key]`.
    ///
    /// Handles the three special tokens: `<<present>>`, `<<absent>>`, `<<uuid>>`.
    fn render_assert_header(&self, out: &mut String, response_var: &str, name: &str, expected: &str) {
        let header_key = name.to_lowercase();
        let header_expr = format!("{response_var}[{}]", ruby_string_literal(&header_key));
        let assertion = match expected {
            "<<present>>" => {
                format!("      expect({header_expr}).not_to be_nil\n")
            }
            "<<absent>>" => {
                format!("      expect({header_expr}).to be_nil\n")
            }
            "<<uuid>>" => {
                format!(
                    "      expect({header_expr}).to match(/\\A[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}\\z/i)\n"
                )
            }
            literal => {
                let ruby_val = ruby_string_literal(literal);
                format!("      expect({header_expr}).to eq({ruby_val})\n")
            }
        };
        out.push_str(&assertion);
    }

    /// Emit a full JSON body equality assertion.
    ///
    /// Plain string bodies are compared as raw text; structured bodies are parsed
    /// with `JSON.parse` and compared as Ruby Hash/Array values.
    fn render_assert_json_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        match expected {
            serde_json::Value::String(s) => {
                let ruby_val = ruby_string_literal(s);
                out.push_str(&format!("      expect({response_var}.body).to eq({ruby_val})\n"));
            }
            _ => {
                let ruby_val = json_to_ruby(expected);
                out.push_str(&format!(
                    "      _body = {response_var}.body && !{response_var}.body.empty? ? JSON.parse({response_var}.body) : nil\n"
                ));
                out.push_str(&format!("      expect(_body).to eq({ruby_val})\n"));
            }
        }
    }

    /// Emit partial body assertions: one `expect(_body[key]).to eq(val)` per field.
    fn render_assert_partial_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            out.push_str(&format!("      _body = JSON.parse({response_var}.body)\n"));
            for (key, val) in obj {
                let ruby_key = ruby_string_literal(key);
                let ruby_val = json_to_ruby(val);
                out.push_str(&format!("      expect(_body[{ruby_key}]).to eq({ruby_val})\n"));
            }
        }
    }

    /// Emit validation-error assertions, checking each expected `msg` against the
    /// parsed body's `errors` array.
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        for err in errors {
            let msg_lit = ruby_string_literal(&err.msg);
            out.push_str(&format!("      _body = JSON.parse({response_var}.body)\n"));
            out.push_str("      _errors = _body['errors'] || []\n");
            out.push_str(&format!(
                "      expect(_errors.map {{ |e| e['msg'] }}).to include({msg_lit})\n"
            ));
        }
    }
}

/// Render an RSpec example for an HTTP server test fixture via the shared driver.
///
/// Delegates to [`client::http_call::render_http_test`] after handling the one
/// Ruby-specific pre-condition: HTTP 101 (WebSocket upgrade) cannot be exercised
/// via `Net::HTTP` and is emitted as a pending `it` block directly.
fn render_http_example(out: &mut String, fixture: &Fixture) {
    // HTTP 101 (WebSocket upgrade) cannot be tested via Net::HTTP.
    // Emit the skip block directly rather than pushing a skip directive through
    // the shared driver, which would require a full `fixture.skip` entry.
    if fixture
        .http
        .as_ref()
        .is_some_and(|h| h.expected_response.status_code == 101)
    {
        if let Some(http) = fixture.http.as_ref() {
            let description = fixture.description.replace('\'', "\\'");
            let method = http.request.method.to_uppercase();
            let path = &http.request.path;
            let rendered = crate::template_env::render(
                "ruby/http_101_skip.jinja",
                minijinja::context! {
                    method => method,
                    path => path,
                    description => description,
                },
            );
            out.push_str(&rendered);
        }
        return;
    }

    client::http_call::render_http_test(out, &RubyTestClientRenderer, fixture);
}

/// Convert an uppercase HTTP method string to Ruby's Net::HTTP class name.
/// Ruby uses title-cased names: Get, Post, Put, Delete, Patch, Head, Options, Trace.
fn http_method_class(method: &str) -> String {
    let mut chars = method.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
    }
}

// ---------------------------------------------------------------------------
// Chat-stream test rendering — block iteration with local aggregation
// ---------------------------------------------------------------------------

/// Render an RSpec example for a `chat_stream` fixture.
///
/// The Ruby binding's `chat_stream` is block-yielding: each yielded value is a
/// `LiterLlm::ChatCompletionChunk`. The codegen builds local aggregator vars
/// (`chunks`, `stream_content`, `stream_complete`, plus optional
/// `last_finish_reason`, `tool_calls_json`, `total_tokens`) inside the block and
/// then emits assertions on those locals — never on response pseudo-fields.
#[allow(clippy::too_many_arguments)]
fn render_chat_stream_example(
    fixture: &Fixture,
    function_name: &str,
    call_receiver: &str,
    args: &[crate::config::ArgMapping],
    options_type: Option<&str>,
    enum_fields: &HashMap<String, String>,
    e2e_config: &E2eConfig,
    client_factory: Option<&str>,
    extra_args: &[String],
) -> String {
    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.replace('\'', "\\'");
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let fixture_id = fixture.id.clone();

    let (mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        call_receiver,
        options_type,
        enum_fields,
        false,
        fixture,
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

    // Detect which aggregators a fixture's assertions actually need so we don't
    // emit unused locals (rubocop trips on assigned-but-unread vars).
    let mut needs_finish_reason = false;
    let mut needs_tool_calls_json = false;
    let mut needs_tool_calls_0_function_name = false;
    let mut needs_total_tokens = false;
    for a in &fixture.assertions {
        if let Some(f) = a.field.as_deref() {
            match f {
                "finish_reason" => needs_finish_reason = true,
                "tool_calls" => needs_tool_calls_json = true,
                "tool_calls[0].function.name" => needs_tool_calls_0_function_name = true,
                "usage.total_tokens" => needs_total_tokens = true,
                _ => {}
            }
        }
    }

    let mut out = String::new();
    out.push_str(&format!("  it '{test_name}: {description}' do\n"));

    // Client construction.
    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    if let Some(cf) = client_factory {
        if has_mock && let Some(key_var) = api_key_var {
            let mock_url_expr = format!("\"#{{ENV['MOCK_SERVER_URL']}}/fixtures/{fixture_id}\"");
            out.push_str(&format!("    api_key = ENV['{key_var}']\n"));
            out.push_str(&format!("    if api_key && !api_key.empty?\n"));
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
    out.push_str("    stream_content = ''.dup\n");
    out.push_str("    stream_complete = false\n");
    if needs_finish_reason {
        out.push_str("    last_finish_reason = nil\n");
    }
    if needs_tool_calls_json {
        out.push_str("    tool_calls_json = nil\n");
    }
    if needs_tool_calls_0_function_name {
        out.push_str("    tool_calls_0_function_name = nil\n");
    }
    if needs_total_tokens {
        out.push_str("    total_tokens = nil\n");
    }
    out.push_str(&format!("    {call_expr} do |chunk|\n"));
    out.push_str("      chunks << chunk\n");
    out.push_str("      choice = chunk.choices && chunk.choices[0]\n");
    out.push_str("      if choice\n");
    out.push_str("        delta = choice.delta\n");
    out.push_str("        if delta && delta.content\n");
    out.push_str("          stream_content << delta.content\n");
    out.push_str("        end\n");
    if needs_finish_reason {
        out.push_str("        if choice.finish_reason\n");
        out.push_str("          last_finish_reason = choice.finish_reason.to_s\n");
        out.push_str("        end\n");
    }
    if needs_tool_calls_json || needs_tool_calls_0_function_name {
        out.push_str("        tcs = delta && delta.tool_calls\n");
        out.push_str("        if tcs && !tcs.empty?\n");
        if needs_tool_calls_json {
            out.push_str(
                "          tool_calls_json ||= tcs.map { |tc| { 'function' => { 'name' => (tc.function && tc.function.name rescue nil) } } }.to_json\n",
            );
        }
        if needs_tool_calls_0_function_name {
            out.push_str(
                "          tool_calls_0_function_name ||= (tcs[0].function && tcs[0].function.name rescue nil)\n",
            );
        }
        out.push_str("        end\n");
    }
    out.push_str("      end\n");
    if needs_total_tokens {
        out.push_str("      if chunk.usage && chunk.usage.total_tokens\n");
        out.push_str("        total_tokens = chunk.usage.total_tokens\n");
        out.push_str("      end\n");
    }
    out.push_str("    end\n");
    out.push_str("    stream_complete = true\n");

    // Render assertions on the local aggregator vars.
    for assertion in &fixture.assertions {
        emit_chat_stream_assertion(&mut out, assertion, e2e_config);
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

    out.push_str("  end\n");
    out
}

/// Map a streaming fixture assertion to an `expect` call on the local aggregator
/// variable produced by [`render_chat_stream_example`]. Pseudo-fields like
/// `chunks` / `stream_content` / `stream_complete` resolve to the in-block locals,
/// not response accessors.
fn emit_chat_stream_assertion(out: &mut String, assertion: &Assertion, _e2e_config: &E2eConfig) {
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

    let (expr, kind) = match field {
        "chunks" => ("chunks", Kind::Chunks),
        "stream_content" => ("stream_content", Kind::Str),
        "stream_complete" => ("stream_complete", Kind::Bool),
        "no_chunks_after_done" => ("stream_complete", Kind::Bool),
        "finish_reason" => ("last_finish_reason", Kind::Str),
        "tool_calls" => ("tool_calls_json", Kind::Json),
        "tool_calls[0].function.name" => ("tool_calls_0_function_name", Kind::Str),
        "usage.total_tokens" => ("total_tokens", Kind::IntTokens),
        _ => ("", Kind::Unsupported),
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
                out.push_str(&format!("    expect({expr}.to_s).to eq({rb_val})\n"));
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

// ---------------------------------------------------------------------------
// Function-call test rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_example(
    fixture: &Fixture,
    function_name: &str,
    call_receiver: &str,
    result_var: &str,
    args: &[crate::config::ArgMapping],
    field_resolver: &FieldResolver,
    options_type: Option<&str>,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    client_factory: Option<&str>,
    extra_args: &[String],
) -> String {
    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.replace('\'', "\\'");
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let fixture_id = fixture.id.clone();

    let (mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        call_receiver,
        options_type,
        enum_fields,
        result_is_simple,
        fixture,
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
            enum_fields,
        );
    }

    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    let has_mock_and_key = has_mock && api_key_var.is_some();
    crate::template_env::render(
        "ruby/test_function.jinja",
        minijinja::context! {
            test_name => test_name,
            description => description,
            expects_error => expects_error,
            setup_lines => setup_lines,
            call_expr => call_expr,
            result_var => result_var,
            assertions_rendered => assertions_rendered,
            has_usable => has_usable,
            client_factory => client_factory,
            fixture_id => fixture_id,
            call_receiver => call_receiver,
            has_mock => has_mock,
            api_key_var => api_key_var,
            has_mock_and_key => has_mock_and_key,
        },
    )
}

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// Returns `(setup_lines, args_string)`.
/// Emit Ruby batch item constructors for BatchBytesItem or BatchFileItem arrays.
fn emit_ruby_batch_item_array(arr: &serde_json::Value, elem_type: &str) -> String {
    if let Some(items) = arr.as_array() {
        let item_strs: Vec<String> = items
            .iter()
            .filter_map(|item| {
                if let Some(obj) = item.as_object() {
                    match elem_type {
                        "BatchBytesItem" => {
                            let content = obj.get("content").and_then(|v| v.as_array());
                            let mime_type = obj.get("mime_type").and_then(|v| v.as_str()).unwrap_or("text/plain");
                            let config = obj.get("config");
                            let content_code = if let Some(arr) = content {
                                let bytes: Vec<String> =
                                    arr.iter().filter_map(|v| v.as_u64().map(|n| n.to_string())).collect();
                                // Pass as Ruby array - Magnus will convert Array<u8> to Vec<u8>
                                format!("[{}]", bytes.join(", "))
                            } else {
                                "[]".to_string()
                            };
                            let config_arg = if let Some(cfg) = config {
                                if cfg.is_null() {
                                    "nil".to_string()
                                } else {
                                    json_to_ruby(cfg)
                                }
                            } else {
                                "nil".to_string()
                            };
                            Some(format!(
                                "Kreuzberg::{}.new(content: {}, mime_type: \"{}\", config: {})",
                                elem_type, content_code, mime_type, config_arg
                            ))
                        }
                        "BatchFileItem" => {
                            let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or("");
                            let config = obj.get("config");
                            let config_arg = if let Some(cfg) = config {
                                if cfg.is_null() {
                                    "nil".to_string()
                                } else {
                                    json_to_ruby(cfg)
                                }
                            } else {
                                "nil".to_string()
                            };
                            Some(format!(
                                "Kreuzberg::{}.new(path: \"{}\", config: {})",
                                elem_type, path, config_arg
                            ))
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .collect();
        format!("[{}]", item_strs.join(", "))
    } else {
        "[]".to_string()
    }
}

fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    call_receiver: &str,
    options_type: Option<&str>,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    fixture: &crate::fixture::Fixture,
) -> (Vec<String>, String) {
    let fixture_id = &fixture.id;
    if args.is_empty() {
        // No args config: pass the whole input only when it's non-empty.
        // Functions with no parameters have empty input and must be called
        // with no arguments — not with `{}` or `nil`.
        let is_empty_input = match input {
            serde_json::Value::Null => true,
            serde_json::Value::Object(m) => m.is_empty(),
            _ => false,
        };
        if is_empty_input {
            return (Vec::new(), String::new());
        }
        return (Vec::new(), json_to_ruby(input));
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    // Track optional args that were skipped; if a later arg is emitted we must back-fill nil
    // to preserve positional correctness (e.g. extract_file(path, nil, config)).
    let mut skipped_optional_count: usize = 0;

    for arg in args {
        if arg.arg_type == "mock_url" {
            // Flush any pending nil placeholders for skipped optionals before this positional arg.
            for _ in 0..skipped_optional_count {
                parts.push("nil".to_string());
            }
            skipped_optional_count = 0;
            if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                setup_lines.push(format!(
                    "{} = ENV.fetch('{env_key}', nil) || \"#{{ENV.fetch('MOCK_SERVER_URL')}}/fixtures/{fixture_id}\"",
                    arg.name,
                ));
            } else {
                setup_lines.push(format!(
                    "{} = \"#{{ENV.fetch('MOCK_SERVER_URL')}}/fixtures/{fixture_id}\"",
                    arg.name,
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        // Handle bytes arguments: load from file if needed
        if arg.arg_type == "bytes" {
            // Flush any pending nil placeholders for skipped optionals before this positional arg.
            for _ in 0..skipped_optional_count {
                parts.push("nil".to_string());
            }
            skipped_optional_count = 0;
            let resolved = resolve_field(input, &arg.field);
            if let Some(s) = resolved.as_str() {
                if is_file_path(s) {
                    // File path: load with File.read and convert to bytes array
                    setup_lines.push(format!("{} = File.read(\"{}\").bytes", arg.name, s));
                } else if is_base64(s) {
                    // Base64: decode it
                    setup_lines.push(format!("{} = Base64.decode64(\"{}\").bytes", arg.name, s));
                } else {
                    // Inline text: encode to binary and convert to bytes array
                    let escaped = ruby_string_literal(s);
                    setup_lines.push(format!("{} = {}.b.bytes", arg.name, escaped));
                }
                parts.push(arg.name.clone());
            } else {
                parts.push("nil".to_string());
            }
            continue;
        }

        // Handle file_path arguments: pass the path string as-is
        if arg.arg_type == "file_path" {
            // Flush any pending nil placeholders for skipped optionals before this positional arg.
            for _ in 0..skipped_optional_count {
                parts.push("nil".to_string());
            }
            skipped_optional_count = 0;
            let resolved = resolve_field(input, &arg.field);
            if let Some(s) = resolved.as_str() {
                let escaped = ruby_string_literal(s);
                parts.push(escaped);
            } else if arg.optional {
                skipped_optional_count += 1;
                continue;
            } else {
                parts.push("''".to_string());
            }
            continue;
        }

        if arg.arg_type == "handle" {
            // Flush any pending nil placeholders for skipped optionals before this positional arg.
            for _ in 0..skipped_optional_count {
                parts.push("nil".to_string());
            }
            skipped_optional_count = 0;
            // Generate a create_engine (or equivalent) call and pass the variable.
            let constructor_name = format!("create_{}", arg.name.to_snake_case());
            let config_value = resolve_field(input, &arg.field);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("{} = {call_receiver}.{constructor_name}(nil)", arg.name,));
            } else {
                let literal = json_to_ruby(config_value);
                let name = &arg.name;
                setup_lines.push(format!("{name}_config = {literal}"));
                setup_lines.push(format!(
                    "{} = {call_receiver}.{constructor_name}({name}_config.to_json)",
                    arg.name,
                    name = name,
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        let resolved = resolve_field(input, &arg.field);
        let val = if resolved.is_null() { None } else { Some(resolved) };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value: defer; emit nil only if a later arg is present.
                skipped_optional_count += 1;
                continue;
            }
            None | Some(serde_json::Value::Null) => {
                // Required arg with no fixture value: flush deferred nils, then pass a default.
                for _ in 0..skipped_optional_count {
                    parts.push("nil".to_string());
                }
                skipped_optional_count = 0;
                let default_val = match arg.arg_type.as_str() {
                    "string" => "''".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "nil".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                // Flush deferred nil placeholders for skipped optional args that precede this one.
                for _ in 0..skipped_optional_count {
                    parts.push("nil".to_string());
                }
                skipped_optional_count = 0;
                // For json_object args with options_type, construct a typed options object.
                // When result_is_simple, the binding accepts a plain Hash (no wrapper class).
                if arg.arg_type == "json_object" && !v.is_null() {
                    // Check for batch item arrays (element_type set to BatchBytesItem/BatchFileItem)
                    if let Some(elem_type) = &arg.element_type {
                        if (elem_type == "BatchBytesItem" || elem_type == "BatchFileItem") && v.is_array() {
                            parts.push(emit_ruby_batch_item_array(v, elem_type));
                            continue;
                        }
                    }
                    // Otherwise handle regular options_type objects
                    if let (Some(opts_type), Some(obj)) = (options_type, v.as_object()) {
                        let kwargs: Vec<String> = obj
                            .iter()
                            .map(|(k, vv)| {
                                let snake_key = k.to_snake_case();
                                let rb_val = if enum_fields.contains_key(k) {
                                    if let Some(s) = vv.as_str() {
                                        let snake_val = s.to_snake_case();
                                        format!("'{snake_val}'")
                                    } else {
                                        json_to_ruby(vv)
                                    }
                                } else {
                                    json_to_ruby(vv)
                                };
                                format!("{snake_key}: {rb_val}")
                            })
                            .collect();
                        if result_is_simple {
                            parts.push(format!("{{{}}}", kwargs.join(", ")));
                        } else {
                            parts.push(format!("{opts_type}.new({})", kwargs.join(", ")));
                        }
                        continue;
                    }
                }
                parts.push(json_to_ruby(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
}

fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    per_call_enum_fields: &HashMap<String, String>,
) {
    // For simple-result methods (e.g. `speech` returning bytes), every field-based
    // assertion targets the result itself — there's no struct to access. Drop
    // length-only assertions onto the result directly and skip anything else.
    if result_is_simple {
        if let Some(f) = &assertion.field {
            if !f.is_empty() {
                match assertion.assertion_type.as_str() {
                    "not_empty" => {
                        out.push_str(&format!("    expect({result_var}.to_s).not_to be_empty\n"));
                        return;
                    }
                    "is_empty" => {
                        out.push_str(&format!("    expect({result_var}.to_s).to be_empty\n"));
                        return;
                    }
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            out.push_str(&format!("    expect({result_var}.length).to eq({rb_val})\n"));
                        }
                        return;
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            out.push_str(&format!("    expect({result_var}.length).to be >= {rb_val}\n"));
                        }
                        return;
                    }
                    _ => {
                        out.push_str(&format!(
                            "    # skipped: field '{f}' not applicable for simple result type\n"
                        ));
                        return;
                    }
                }
            }
        }
    }
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct attribute accesses on the result.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content" => {
                let pred = format!("({result_var}.chunks || []).all? {{ |c| c.content && !c.content.empty? }}");
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        out.push_str(&format!("    expect({pred}).to be(true)\n"));
                    }
                    "is_false" => {
                        out.push_str(&format!("    expect({pred}).to be(false)\n"));
                    }
                    _ => {
                        out.push_str(&format!(
                            "    # skipped: unsupported assertion type on synthetic field '{f}'\n"
                        ));
                    }
                }
                return;
            }
            "chunks_have_embeddings" => {
                let pred =
                    format!("({result_var}.chunks || []).all? {{ |c| !c.embedding.nil? && !c.embedding.empty? }}");
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        out.push_str(&format!("    expect({pred}).to be(true)\n"));
                    }
                    "is_false" => {
                        out.push_str(&format!("    expect({pred}).to be(false)\n"));
                    }
                    _ => {
                        out.push_str(&format!(
                            "    # skipped: unsupported assertion type on synthetic field '{f}'\n"
                        ));
                    }
                }
                return;
            }
            // ---- EmbedResponse virtual fields ----
            // embed_texts returns Array<Array<Float>> in Ruby — no wrapper struct.
            // result_var is the embedding matrix; use it directly.
            "embeddings" => {
                match assertion.assertion_type.as_str() {
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            out.push_str(&format!("    expect({result_var}.length).to eq({rb_val})\n"));
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            out.push_str(&format!("    expect({result_var}.length).to be >= {rb_val}\n"));
                        }
                    }
                    "not_empty" => {
                        out.push_str(&format!("    expect({result_var}).not_to be_empty\n"));
                    }
                    "is_empty" => {
                        out.push_str(&format!("    expect({result_var}).to be_empty\n"));
                    }
                    _ => {
                        out.push_str("    # skipped: unsupported assertion type on synthetic field 'embeddings'\n");
                    }
                }
                return;
            }
            "embedding_dimensions" => {
                let expr = format!("({result_var}.empty? ? 0 : {result_var}[0].length)");
                match assertion.assertion_type.as_str() {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            out.push_str(&format!("    expect({expr}).to eq({rb_val})\n"));
                        }
                    }
                    "greater_than" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            out.push_str(&format!("    expect({expr}).to be > {rb_val}\n"));
                        }
                    }
                    _ => {
                        out.push_str(
                            "    # skipped: unsupported assertion type on synthetic field 'embedding_dimensions'\n",
                        );
                    }
                }
                return;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                let pred = match f.as_str() {
                    "embeddings_valid" => {
                        format!("{result_var}.all? {{ |e| !e.empty? }}")
                    }
                    "embeddings_finite" => {
                        format!("{result_var}.all? {{ |e| e.all? {{ |v| v.finite? }} }}")
                    }
                    "embeddings_non_zero" => {
                        format!("{result_var}.all? {{ |e| e.any? {{ |v| v != 0.0 }} }}")
                    }
                    "embeddings_normalized" => {
                        format!("{result_var}.all? {{ |e| n = e.sum {{ |v| v * v }}; (n - 1.0).abs < 1e-3 }}")
                    }
                    _ => unreachable!(),
                };
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        out.push_str(&format!("    expect({pred}).to be(true)\n"));
                    }
                    "is_false" => {
                        out.push_str(&format!("    expect({pred}).to be(false)\n"));
                    }
                    _ => {
                        out.push_str(&format!(
                            "    # skipped: unsupported assertion type on synthetic field '{f}'\n"
                        ));
                    }
                }
                return;
            }
            // ---- keywords / keywords_count ----
            // Ruby ExtractionResult does not expose extracted_keywords; skip.
            "keywords" | "keywords_count" => {
                out.push_str(&format!(
                    "    # skipped: field '{f}' not available on Ruby ExtractionResult\n"
                ));
                return;
            }
            _ => {}
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            out.push_str(&format!("    # skipped: field '{f}' not available on result type\n"));
            return;
        }
    }

    // When result_is_simple, skip assertions that reference non-content fields.
    if result_is_simple {
        if let Some(f) = &assertion.field {
            let f_lower = f.to_lowercase();
            if !f.is_empty()
                && f_lower != "content"
                && (f_lower.starts_with("metadata")
                    || f_lower.starts_with("document")
                    || f_lower.starts_with("structure"))
            {
                return;
            }
        }
    }

    // result_is_simple: treat the result itself as the content string, but only
    // when there is no explicit field (or the field is "content"). Count/length
    // assertions on named fields (e.g. "warnings") must still walk the field path.
    let field_expr = match &assertion.field {
        Some(f) if !f.is_empty() && (!result_is_simple || !f.eq_ignore_ascii_case("content")) => {
            field_resolver.accessor(f, "ruby", result_var)
        }
        _ => result_var.to_string(),
    };

    // For string equality, strip trailing whitespace to handle trailing newlines
    // from the converter. Ruby enum fields (Magnus binds Rust enums as Symbols),
    // are coerced to String via .to_s so `eq("stop")` matches `:stop`. Look up the
    // field in both the global `[crates.e2e] fields_enum` set AND the per-call
    // override `[crates.e2e.calls.<x>.overrides.<lang>] enum_fields = { ... }` —
    // downstream config that already labels e.g. `status = "BatchStatus"` for the
    // Java/C#/Python sides should apply here too without a Ruby-only duplicate.
    let field_is_enum = assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        e2e_config.fields_enum.contains(f)
            || e2e_config.fields_enum.contains(resolved)
            || per_call_enum_fields.contains_key(f)
            || per_call_enum_fields.contains_key(resolved)
    });
    let stripped_field_expr = if result_is_simple {
        format!("{field_expr}.to_s.strip")
    } else if field_is_enum {
        format!("{field_expr}.to_s")
    } else {
        field_expr.clone()
    };

    // Detect whether the assertion field resolves to an array type so that
    // contains assertions can iterate items instead of calling .to_s on the array.
    let field_is_array = assertion
        .field
        .as_deref()
        .filter(|f| !f.is_empty())
        .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let is_boolean_val = expected.as_bool().is_some();
                let bool_val = expected
                    .as_bool()
                    .map(|b| if b { "true" } else { "false" })
                    .unwrap_or("");
                let rb_val = json_to_ruby(expected);

                let rendered = crate::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "equals",
                        stripped_field_expr => stripped_field_expr.clone(),
                        is_boolean_val => is_boolean_val,
                        bool_val => bool_val,
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let rendered = crate::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains",
                        field_expr => field_expr.clone(),
                        field_is_array => field_is_array && expected.is_string(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                let values_list: Vec<String> = values.iter().map(json_to_ruby).collect();
                let rendered = crate::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains_all",
                        field_expr => field_expr.clone(),
                        field_is_array => field_is_array,
                        values_list => values_list,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let rendered = crate::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "not_contains",
                        field_expr => field_expr.clone(),
                        field_is_array => field_is_array && expected.is_string(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_empty" => {
            let rendered = crate::template_env::render(
                "ruby/assertion.jinja",
                minijinja::context! {
                    assertion_type => "not_empty",
                    field_expr => field_expr.clone(),
                },
            );
            out.push_str(&rendered);
        }
        "is_empty" => {
            let rendered = crate::template_env::render(
                "ruby/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_empty",
                    field_expr => field_expr.clone(),
                },
            );
            out.push_str(&rendered);
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(json_to_ruby).collect();
                let rendered = crate::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains_any",
                        field_expr => field_expr.clone(),
                        values_list => items,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                let rendered = crate::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "greater_than",
                        field_expr => field_expr.clone(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                let rendered = crate::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "less_than",
                        field_expr => field_expr.clone(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                let rendered = crate::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "greater_than_or_equal",
                        field_expr => field_expr.clone(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                let rendered = crate::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "less_than_or_equal",
                        field_expr => field_expr.clone(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let rendered = crate::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "starts_with",
                        field_expr => field_expr.clone(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let rendered = crate::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "ends_with",
                        field_expr => field_expr.clone(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "min_length" | "max_length" | "count_min" | "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let rendered = crate::template_env::render(
                        "ruby/assertion.jinja",
                        minijinja::context! {
                            assertion_type => assertion.assertion_type.as_str(),
                            field_expr => field_expr.clone(),
                            check_n => n,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
        }
        "is_true" => {
            let rendered = crate::template_env::render(
                "ruby/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_true",
                    field_expr => field_expr.clone(),
                },
            );
            out.push_str(&rendered);
        }
        "is_false" => {
            let rendered = crate::template_env::render(
                "ruby/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_false",
                    field_expr => field_expr.clone(),
                },
            );
            out.push_str(&rendered);
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                // Derive call_receiver for module-level helper calls.
                let lang = "ruby";
                let call = &e2e_config.call;
                let overrides = call.overrides.get(lang);
                let module_path = overrides
                    .and_then(|o| o.module.as_ref())
                    .cloned()
                    .unwrap_or_else(|| call.module.clone());
                let call_receiver = ruby_module_name(&module_path);

                let call_expr =
                    build_ruby_method_call(&call_receiver, result_var, method_name, assertion.args.as_ref());
                let check = assertion.check.as_deref().unwrap_or("is_true");

                let (check_val_str, is_boolean_check, bool_check_val, check_n_val) = match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let is_bool = val.as_bool().is_some();
                            let bool_str = val.as_bool().map(|b| if b { "true" } else { "false" }).unwrap_or("");
                            let rb_val = json_to_ruby(val);
                            (rb_val, is_bool, bool_str.to_string(), 0)
                        } else {
                            (String::new(), false, String::new(), 0)
                        }
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            (json_to_ruby(val), false, String::new(), 0)
                        } else {
                            (String::new(), false, String::new(), 0)
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            (String::new(), false, String::new(), n)
                        } else {
                            (String::new(), false, String::new(), 0)
                        }
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            (json_to_ruby(val), false, String::new(), 0)
                        } else {
                            (String::new(), false, String::new(), 0)
                        }
                    }
                    _ => (String::new(), false, String::new(), 0),
                };

                let rendered = crate::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "method_result",
                        call_expr => call_expr,
                        check => check,
                        check_val => check_val_str,
                        is_boolean_check => is_boolean_check,
                        bool_check_val => bool_check_val,
                        check_n => check_n_val,
                    },
                );
                out.push_str(&rendered);
            } else {
                panic!("Ruby e2e generator: method_result assertion missing 'method' field");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let rendered = crate::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "matches_regex",
                        field_expr => field_expr.clone(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_error" => {
            // Already handled by the call succeeding without exception.
        }
        "error" => {
            // Handled at the example level.
        }
        other => {
            panic!("Ruby e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Build a Ruby call expression for a `method_result` assertion on a tree-sitter Tree.
/// Maps method names to the appropriate Ruby method or module-function calls.
fn build_ruby_method_call(
    call_receiver: &str,
    result_var: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
) -> String {
    match method_name {
        "root_child_count" => format!("{result_var}.root_node.child_count"),
        "root_node_type" => format!("{result_var}.root_node.type"),
        "named_children_count" => format!("{result_var}.root_node.named_child_count"),
        "has_error_nodes" => format!("{call_receiver}.tree_has_error_nodes({result_var})"),
        "error_count" | "tree_error_count" => format!("{call_receiver}.tree_error_count({result_var})"),
        "tree_to_sexp" => format!("{call_receiver}.tree_to_sexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{call_receiver}.tree_contains_node_type({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{call_receiver}.find_nodes_by_type({result_var}, \"{node_type}\")")
        }
        "run_query" => {
            let query_source = args
                .and_then(|a| a.get("query_source"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let language = args
                .and_then(|a| a.get("language"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{call_receiver}.run_query({result_var}, \"{language}\", \"{query_source}\", source)")
        }
        _ => format!("{result_var}.{method_name}"),
    }
}

/// Convert a module path (e.g., "html_to_markdown") to Ruby PascalCase module name
/// (e.g., "HtmlToMarkdown").
fn ruby_module_name(module_path: &str) -> String {
    use heck::ToUpperCamelCase;
    module_path.to_upper_camel_case()
}

/// Convert a `serde_json::Value` to a Ruby literal string, preferring single quotes.
fn json_to_ruby(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => ruby_string_literal(s),
        serde_json::Value::Bool(true) => "true".to_string(),
        serde_json::Value::Bool(false) => "false".to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "nil".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_ruby).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{} => {}", ruby_string_literal(k), json_to_ruby(v)))
                .collect();
            format!("{{ {} }}", items.join(", "))
        }
    }
}

// ---------------------------------------------------------------------------
// Visitor generation
// ---------------------------------------------------------------------------

/// Build a Ruby visitor object and add setup lines. Returns the visitor expression.
fn build_ruby_visitor(setup_lines: &mut Vec<String>, visitor_spec: &crate::fixture::VisitorSpec) -> String {
    setup_lines.push("visitor = Class.new do".to_string());
    for (method_name, action) in &visitor_spec.callbacks {
        emit_ruby_visitor_method(setup_lines, method_name, action);
    }
    setup_lines.push("end.new".to_string());
    "visitor".to_string()
}

/// Emit a Ruby visitor method for a callback action.
fn emit_ruby_visitor_method(setup_lines: &mut Vec<String>, method_name: &str, action: &CallbackAction) {
    let params = match method_name {
        "visit_link" => "ctx, href, text, title",
        "visit_image" => "ctx, src, alt, title",
        "visit_heading" => "ctx, level, text, id",
        "visit_code_block" => "ctx, lang, code",
        "visit_code_inline"
        | "visit_strong"
        | "visit_emphasis"
        | "visit_strikethrough"
        | "visit_underline"
        | "visit_subscript"
        | "visit_superscript"
        | "visit_mark"
        | "visit_button"
        | "visit_summary"
        | "visit_figcaption"
        | "visit_definition_term"
        | "visit_definition_description" => "ctx, text",
        "visit_text" => "ctx, text",
        "visit_list_item" => "ctx, ordered, marker, text",
        "visit_blockquote" => "ctx, content, depth",
        "visit_table_row" => "ctx, cells, is_header",
        "visit_custom_element" => "ctx, tag_name, html",
        "visit_form" => "ctx, action_url, method",
        "visit_input" => "ctx, input_type, name, value",
        "visit_audio" | "visit_video" | "visit_iframe" => "ctx, src",
        "visit_details" => "ctx, is_open",
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => "ctx, output",
        "visit_list_start" => "ctx, ordered",
        "visit_list_end" => "ctx, ordered, output",
        _ => "ctx",
    };

    // Pre-compute action type and values
    let (action_type, action_value) = match action {
        CallbackAction::Skip => ("skip", String::new()),
        CallbackAction::Continue => ("continue", String::new()),
        CallbackAction::PreserveHtml => ("preserve_html", String::new()),
        CallbackAction::Custom { output } => {
            let escaped = ruby_string_literal(output);
            ("custom", escaped)
        }
        CallbackAction::CustomTemplate { template } => {
            let interpolated = ruby_template_to_interpolation(template);
            ("custom", format!("\"{interpolated}\""))
        }
    };

    let rendered = crate::template_env::render(
        "ruby/visitor_method.jinja",
        minijinja::context! {
            method_name => method_name,
            params => params,
            action_type => action_type,
            action_value => action_value,
        },
    );
    for line in rendered.lines() {
        setup_lines.push(line.to_string());
    }
}

/// Classify a fixture string value that maps to a `bytes` argument.
///
/// Returns true if the value looks like a file path (e.g. "pdf/fake_memo.pdf").
/// File paths have the pattern: alphanumeric/something.extension
fn is_file_path(s: &str) -> bool {
    if s.starts_with('<') || s.starts_with('{') || s.starts_with('[') || s.contains(' ') {
        return false;
    }

    let first = s.chars().next().unwrap_or('\0');
    if first.is_ascii_alphanumeric() || first == '_' {
        if let Some(slash_pos) = s.find('/') {
            if slash_pos > 0 {
                let after_slash = &s[slash_pos + 1..];
                if after_slash.contains('.') && !after_slash.is_empty() {
                    return true;
                }
            }
        }
    }

    false
}

/// Check if a string looks like base64-encoded data.
/// If it's not a file path or inline text, assume it's base64.
fn is_base64(s: &str) -> bool {
    if s.starts_with('<') || s.starts_with('{') || s.starts_with('[') || s.contains(' ') {
        return false;
    }

    if is_file_path(s) {
        return false;
    }

    true
}
