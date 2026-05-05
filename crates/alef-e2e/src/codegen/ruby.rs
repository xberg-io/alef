//! Ruby e2e test generator using RSpec.
//!
//! Generates `e2e/ruby/Gemfile` and `spec/{category}_spec.rb` files from
//! JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

use crate::codegen::resolve_field;
use crate::config::E2eConfig;
use crate::escape::{ruby_string_literal, sanitize_filename, sanitize_ident};
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
        let has_http_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| f.is_http_test());

        // Check if any fixture uses file_path or bytes args (needs chdir to test_documents).
        let has_file_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
            let cc = e2e_config.resolve_call(f.call.as_deref());
            cc.args
                .iter()
                .any(|a| a.arg_type == "file_path" || a.arg_type == "bytes")
        });

        // Always generate spec/spec_helper.rb when file-based or HTTP fixtures are present.
        if has_file_fixtures || has_http_fixtures {
            files.push(GeneratedFile {
                path: output_base.join("spec").join("spec_helper.rb"),
                content: render_spec_helper(has_file_fixtures, has_http_fixtures),
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
            );
            // Skip the entire file if no fixture in this category produces output.
            let has_any_output = active.iter().any(|f| {
                // HTTP tests always produce output.
                if f.is_http_test() {
                    return true;
                }
                let expects_error = f.assertions.iter().any(|a| a.assertion_type == "error");
                expects_error || has_usable_assertion(f, &field_resolver_pre, result_is_simple)
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
    format!(
        "# frozen_string_literal: true\n\
         \n\
         source 'https://rubygems.org'\n\
         \n\
         {gem_line}\n\
         gem 'rspec', '{rspec}'\n\
         gem 'rubocop', '{rubocop}'\n\
         gem 'rubocop-rspec', '{rubocop_rspec}'\n\
         gem 'faraday', '{faraday}'\n",
        rspec = tv::gem::RSPEC_E2E,
        rubocop = tv::gem::RUBOCOP_E2E,
        rubocop_rspec = tv::gem::RUBOCOP_RSPEC_E2E,
        faraday = tv::gem::FARADAY,
    )
}

fn render_spec_helper(has_file_fixtures: bool, has_http_fixtures: bool) -> String {
    let header = hash::header(CommentStyle::Hash);
    let mut out = header;
    out.push_str("# frozen_string_literal: true\n");

    if has_file_fixtures {
        out.push_str(
            r#"
# Change to the test_documents directory so that fixture file paths like
# "pdf/fake_memo.pdf" resolve correctly when running rspec from e2e/ruby/.
# spec_helper.rb lives in e2e/ruby/spec/; test_documents lives at the
# repository root, three directories up: spec/ -> e2e/ruby/ -> e2e/ -> root.
_test_documents = File.expand_path('../../../test_documents', __dir__)
Dir.chdir(_test_documents) if Dir.exist?(_test_documents)
"#,
        );
    }

    if has_http_fixtures {
        out.push_str(
            r#"
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
    url = stdout.readline.strip.split('=', 2).last
    ENV['MOCK_SERVER_URL'] = url
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
    r#"# Generated by alef e2e — do not edit.
AllCops:
  NewCops: enable
  TargetRubyVersion: 3.2
  SuggestExtensions: false

plugins:
  - rubocop-rspec

# --- Justified suppressions for generated test code ---

# Generated tests are verbose by nature (setup + multiple assertions).
Metrics/BlockLength:
  Enabled: false
Metrics/MethodLength:
  Enabled: false
Layout/LineLength:
  Enabled: false

# Generated tests use multiple assertions per example for thorough verification.
RSpec/MultipleExpectations:
  Enabled: false
RSpec/ExampleLength:
  Enabled: false

# Generated tests describe categories as strings, not classes.
RSpec/DescribeClass:
  Enabled: false

# Fixture-driven tests may produce identical assertion bodies for different inputs.
RSpec/RepeatedExample:
  Enabled: false

# Error-handling tests use bare raise_error (exception type not known at generation time).
RSpec/UnspecifiedException:
  Enabled: false
"#
    .to_string()
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

    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "# frozen_string_literal: true");
    let _ = writeln!(out);

    // Require the gem (single quotes).
    let require_name = if module_path.is_empty() { gem_name } else { module_path };
    let _ = writeln!(out, "require '{}'", require_name.replace('-', "_"));
    let _ = writeln!(out, "require 'json'");

    let has_http = fixtures.iter().any(|f| f.is_http_test());
    if needs_spec_helper || has_http {
        // spec_helper sets up Dir.chdir and/or mock server.
        let _ = writeln!(out, "require_relative 'spec_helper'");
    }
    let _ = writeln!(out);

    // Build the Ruby module/class qualifier for calls.
    let call_receiver = class_name
        .map(|s| s.to_string())
        .unwrap_or_else(|| ruby_module_name(module_path));

    let _ = writeln!(out, "RSpec.describe '{}' do", category);

    // Emit a shared helper for array field contains assertions — extracts text
    // representations from each item so `.include?` works on struct arrays.
    let has_array_contains = fixtures.iter().any(|fixture| {
        fixture.assertions.iter().any(|a| {
            matches!(a.assertion_type.as_str(), "contains" | "contains_all" | "not_contains")
                && a.field
                    .as_deref()
                    .is_some_and(|f| !f.is_empty() && field_resolver.is_array(field_resolver.resolve(f)))
        })
    });
    if has_array_contains {
        let _ = writeln!(out);
        let _ = writeln!(out, "  def alef_e2e_item_texts(item)");
        let _ = writeln!(
            out,
            "    [:kind, :name, :signature, :path, :alias, :text, :source].filter_map do |attr|"
        );
        let _ = writeln!(out, "      item.respond_to?(attr) ? item.send(attr).to_s : nil");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
    }

    // Emit a shared client helper when there are HTTP tests.
    if has_http {
        let _ = writeln!(
            out,
            "  let(:mock_server_url) {{ ENV.fetch('MOCK_SERVER_URL', 'http://localhost:8080') }}"
        );
        let _ = writeln!(out);
    }

    let mut first = true;
    for fixture in fixtures {
        if !first {
            let _ = writeln!(out);
        }
        first = false;

        if fixture.http.is_some() {
            render_http_example(&mut out, fixture);
        } else {
            // Non-HTTP fixtures (WebSocket, SSE, gRPC, etc.) that have no usable assertions
            // cannot be tested via Net::HTTP. Emit a pending example instead.
            let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
            let has_usable = has_usable_assertion(fixture, field_resolver, result_is_simple);
            if !expects_error && !has_usable {
                let test_name = sanitize_ident(&fixture.id);
                let description = fixture.description.replace('\'', "\\'");
                let _ = writeln!(out, "  it '{test_name}: {description}' do");
                let _ = writeln!(out, "    skip 'Non-HTTP fixture cannot be tested via Net::HTTP'");
                let _ = writeln!(out, "  end");
            } else {
                // Resolve per-fixture call config (supports named calls via fixture.call field).
                let fixture_call = e2e_config.resolve_call(fixture.call.as_deref());
                let fixture_call_overrides = fixture_call.overrides.get("ruby");
                let fixture_function_name = fixture_call_overrides
                    .and_then(|o| o.function.as_ref())
                    .cloned()
                    .unwrap_or_else(|| fixture_call.function.clone());
                let fixture_result_var = &fixture_call.result_var;
                let fixture_args = &fixture_call.args;
                // Per-fixture client_factory: prefer fixture-level override, then default.
                let fixture_client_factory = fixture_call_overrides
                    .and_then(|o| o.client_factory.as_deref())
                    .or(client_factory);
                // Per-fixture options_type: prefer fixture-level override, then global default.
                let fixture_options_type = fixture_call_overrides
                    .and_then(|o| o.options_type.as_deref())
                    .or(options_type);
                render_example(
                    &mut out,
                    fixture,
                    &fixture_function_name,
                    &call_receiver,
                    fixture_result_var,
                    fixture_args,
                    field_resolver,
                    fixture_options_type,
                    enum_fields,
                    result_is_simple,
                    e2e_config,
                    fixture_client_factory,
                );
            }
        }
    }

    let _ = writeln!(out, "end");
    out
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
        let _ = writeln!(out, "  describe '{fn_name}' do");
        if let Some(reason) = skip_reason {
            let _ = writeln!(out, "    it '{escaped_description}' do");
            let _ = writeln!(out, "      skip '{reason}'");
            // NB: the shared driver calls render_test_close immediately after render_test_open
            // for skipped fixtures, so the closing `end`s are emitted there.
        } else {
            let _ = writeln!(out, "    it '{escaped_description}' do");
        }
    }

    /// Close the inner `it` block and the outer `describe` block.
    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
    }

    /// Emit a `Net::HTTP` request to the mock server using the path from `ctx`.
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();
        let method_class = http_method_class(&method);
        let _ = writeln!(out, "      require 'net/http'");
        let _ = writeln!(out, "      require 'uri'");
        let _ = writeln!(out, "      require 'json'");
        let _ = writeln!(out, "      _uri = URI.parse(\"#{{mock_server_url}}{}\")", ctx.path);
        let _ = writeln!(out, "      _http = Net::HTTP.new(_uri.host, _uri.port)");
        let _ = writeln!(out, "      _http.use_ssl = _uri.scheme == 'https'");
        let _ = writeln!(out, "      _req = Net::HTTP::{method_class}.new(_uri.request_uri)");

        let has_body = ctx
            .body
            .is_some_and(|b| !matches!(b, serde_json::Value::String(s) if s.is_empty()));
        if has_body {
            let ruby_body = json_to_ruby(ctx.body.unwrap());
            let _ = writeln!(out, "      _req.body = {ruby_body}.to_json");
            let _ = writeln!(out, "      _req['Content-Type'] = 'application/json'");
        }

        for (k, v) in ctx.headers {
            // Skip Content-Type when already set from the body above.
            if has_body && k.to_lowercase() == "content-type" {
                continue;
            }
            let rk = ruby_string_literal(k);
            let rv = ruby_string_literal(v);
            let _ = writeln!(out, "      _req[{rk}] = {rv}");
        }

        let _ = writeln!(out, "      {} = _http.request(_req)", ctx.response_var);
    }

    /// Emit `expect(response.code.to_i).to eq(status)`.
    ///
    /// Net::HTTP returns the HTTP status as a `String`; `.to_i` converts it for
    /// comparison with the integer literal from the fixture.
    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        let _ = writeln!(out, "      expect({response_var}.code.to_i).to eq({status})");
    }

    /// Emit a header assertion using `response[header_key]`.
    ///
    /// Handles the three special tokens: `<<present>>`, `<<absent>>`, `<<uuid>>`.
    fn render_assert_header(&self, out: &mut String, response_var: &str, name: &str, expected: &str) {
        let header_key = name.to_lowercase();
        let header_expr = format!("{response_var}[{}]", ruby_string_literal(&header_key));
        match expected {
            "<<present>>" => {
                let _ = writeln!(out, "      expect({header_expr}).not_to be_nil");
            }
            "<<absent>>" => {
                let _ = writeln!(out, "      expect({header_expr}).to be_nil");
            }
            "<<uuid>>" => {
                let _ = writeln!(
                    out,
                    "      expect({header_expr}).to match(/\\A[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}\\z/i)"
                );
            }
            literal => {
                let ruby_val = ruby_string_literal(literal);
                let _ = writeln!(out, "      expect({header_expr}).to eq({ruby_val})");
            }
        }
    }

    /// Emit a full JSON body equality assertion.
    ///
    /// Plain string bodies are compared as raw text; structured bodies are parsed
    /// with `JSON.parse` and compared as Ruby Hash/Array values.
    fn render_assert_json_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        match expected {
            serde_json::Value::String(s) => {
                let ruby_val = ruby_string_literal(s);
                let _ = writeln!(out, "      expect({response_var}.body).to eq({ruby_val})");
            }
            _ => {
                let ruby_val = json_to_ruby(expected);
                let _ = writeln!(
                    out,
                    "      _body = {response_var}.body && !{response_var}.body.empty? ? JSON.parse({response_var}.body) : nil"
                );
                let _ = writeln!(out, "      expect(_body).to eq({ruby_val})");
            }
        }
    }

    /// Emit partial body assertions: one `expect(_body[key]).to eq(val)` per field.
    fn render_assert_partial_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let _ = writeln!(out, "      _body = JSON.parse({response_var}.body)");
            for (key, val) in obj {
                let ruby_key = ruby_string_literal(key);
                let ruby_val = json_to_ruby(val);
                let _ = writeln!(out, "      expect(_body[{ruby_key}]).to eq({ruby_val})");
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
            let _ = writeln!(out, "      _body = JSON.parse({response_var}.body)");
            let _ = writeln!(out, "      _errors = _body['errors'] || []");
            let _ = writeln!(
                out,
                "      expect(_errors.map {{ |e| e['msg'] }}).to include({msg_lit})"
            );
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
            let _ = writeln!(out, "  describe '{method} {path}' do");
            let _ = writeln!(out, "    it '{description}' do");
            let _ = writeln!(
                out,
                "      skip 'HTTP 101 WebSocket upgrade cannot be tested via Net::HTTP'"
            );
            let _ = writeln!(out, "    end");
            let _ = writeln!(out, "  end");
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
// Function-call test rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_example(
    out: &mut String,
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
) {
    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.replace('\'', "\\'");
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let (mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        call_receiver,
        options_type,
        enum_fields,
        result_is_simple,
        &fixture.id,
    );

    // Build visitor if present and add to setup
    let mut visitor_arg = String::new();
    if let Some(visitor_spec) = &fixture.visitor {
        visitor_arg = build_ruby_visitor(&mut setup_lines, visitor_spec);
    }

    let final_args = if visitor_arg.is_empty() {
        args_str
    } else if args_str.is_empty() {
        visitor_arg
    } else {
        format!("{args_str}, {visitor_arg}")
    };

    // When client_factory is configured, create a client instance and call methods on it.
    let call_expr = if client_factory.is_some() {
        format!("client.{function_name}({final_args})")
    } else {
        format!("{call_receiver}.{function_name}({final_args})")
    };

    let _ = writeln!(out, "  it '{test_name}: {description}' do");

    // Emit client creation before setup lines when using client_factory.
    if let Some(factory) = client_factory {
        let fixture_id = &fixture.id;
        let _ = writeln!(
            out,
            "    client = {call_receiver}.{factory}('test-key', ENV.fetch('MOCK_SERVER_URL') + '/fixtures/{fixture_id}')"
        );
    }

    for line in &setup_lines {
        let _ = writeln!(out, "    {line}");
    }

    if expects_error {
        let _ = writeln!(out, "    expect {{ {call_expr} }}.to raise_error");
        let _ = writeln!(out, "  end");
        return;
    }

    // Check if any non-error assertion actually uses the result variable.
    let has_usable = has_usable_assertion(fixture, field_resolver, result_is_simple);
    let _ = writeln!(out, "    {result_var} = {call_expr}");

    for assertion in &fixture.assertions {
        render_assertion(out, assertion, result_var, field_resolver, result_is_simple, e2e_config);
    }

    // When all assertions were skipped (fields unavailable), the example has no
    // expect() calls, which triggers rubocop's RSpec/NoExpectationExample cop.
    // Emit a minimal placeholder expectation so rubocop is satisfied.
    if !has_usable {
        let _ = writeln!(out, "    expect({result_var}).not_to be_nil");
    }

    let _ = writeln!(out, "  end");
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
                            let content_code = if let Some(arr) = content {
                                let bytes: Vec<String> =
                                    arr.iter().filter_map(|v| v.as_u64().map(|n| n.to_string())).collect();
                                format!("[{}].pack('C*')", bytes.join(", "))
                            } else {
                                "''.b".to_string()
                            };
                            Some(format!(
                                "Kreuzberg::{}.new(content: {}, mime_type: \"{}\")",
                                elem_type, content_code, mime_type
                            ))
                        }
                        "BatchFileItem" => {
                            let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or("");
                            Some(format!("Kreuzberg::{}.new(path: \"{}\")", elem_type, path))
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
    fixture_id: &str,
) -> (Vec<String>, String) {
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
            setup_lines.push(format!(
                "{} = \"#{{ENV.fetch('MOCK_SERVER_URL')}}/fixtures/{fixture_id}\"",
                arg.name,
            ));
            parts.push(arg.name.clone());
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
) {
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct attribute accesses on the result.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content" => {
                let pred = format!("({result_var}.chunks || []).all? {{ |c| c.content && !c.content.empty? }}");
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "    expect({pred}).to be(true)");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "    expect({pred}).to be(false)");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "    # skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            "chunks_have_embeddings" => {
                let pred =
                    format!("({result_var}.chunks || []).all? {{ |c| !c.embedding.nil? && !c.embedding.empty? }}");
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "    expect({pred}).to be(true)");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "    expect({pred}).to be(false)");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "    # skipped: unsupported assertion type on synthetic field '{f}'"
                        );
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
                            let _ = writeln!(out, "    expect({result_var}.length).to eq({rb_val})");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            let _ = writeln!(out, "    expect({result_var}.length).to be >= {rb_val}");
                        }
                    }
                    "not_empty" => {
                        let _ = writeln!(out, "    expect({result_var}).not_to be_empty");
                    }
                    "is_empty" => {
                        let _ = writeln!(out, "    expect({result_var}).to be_empty");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "    # skipped: unsupported assertion type on synthetic field 'embeddings'"
                        );
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
                            let _ = writeln!(out, "    expect({expr}).to eq({rb_val})");
                        }
                    }
                    "greater_than" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            let _ = writeln!(out, "    expect({expr}).to be > {rb_val}");
                        }
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "    # skipped: unsupported assertion type on synthetic field 'embedding_dimensions'"
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
                        let _ = writeln!(out, "    expect({pred}).to be(true)");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "    expect({pred}).to be(false)");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "    # skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            // ---- keywords / keywords_count ----
            // Ruby ExtractionResult does not expose extracted_keywords; skip.
            "keywords" | "keywords_count" => {
                let _ = writeln!(out, "    # skipped: field '{f}' not available on Ruby ExtractionResult");
                return;
            }
            _ => {}
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "    # skipped: field '{f}' not available on result type");
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

    let field_expr = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "ruby", result_var),
            _ => result_var.to_string(),
        }
    };

    // For string equality, strip trailing whitespace to handle trailing newlines
    // from the converter.
    let stripped_field_expr = if result_is_simple {
        format!("{field_expr}.to_s.strip")
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
                // Use be(true)/be(false) for booleans (RSpec/BeEq).
                if let Some(b) = expected.as_bool() {
                    let _ = writeln!(out, "    expect({stripped_field_expr}).to be({b})");
                } else {
                    let rb_val = json_to_ruby(expected);
                    let _ = writeln!(out, "    expect({stripped_field_expr}).to eq({rb_val})");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                if field_is_array && expected.is_string() {
                    // Array of structs: check if any item's text representation contains the value.
                    let _ = writeln!(
                        out,
                        "    expect({field_expr}.any? {{ |item| alef_e2e_item_texts(item).any? {{ |t| t.include?({rb_val}) }} }}).to be(true)"
                    );
                } else {
                    // Use .to_s to handle both String and Symbol (enum) fields
                    let _ = writeln!(out, "    expect({field_expr}.to_s).to include({rb_val})");
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let rb_val = json_to_ruby(val);
                    if field_is_array && val.is_string() {
                        let _ = writeln!(
                            out,
                            "    expect({field_expr}.any? {{ |item| alef_e2e_item_texts(item).any? {{ |t| t.include?({rb_val}) }} }}).to be(true)"
                        );
                    } else {
                        let _ = writeln!(out, "    expect({field_expr}.to_s).to include({rb_val})");
                    }
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                if field_is_array && expected.is_string() {
                    let _ = writeln!(
                        out,
                        "    expect({field_expr}.any? {{ |item| alef_e2e_item_texts(item).any? {{ |t| t.include?({rb_val}) }} }}).to be(false)"
                    );
                } else {
                    let _ = writeln!(out, "    expect({field_expr}.to_s).not_to include({rb_val})");
                }
            }
        }
        "not_empty" => {
            if result_is_simple {
                let _ = writeln!(out, "    expect({field_expr}.to_s).not_to be_empty");
            } else {
                let _ = writeln!(out, "    expect({field_expr}).not_to be_empty");
            }
        }
        "is_empty" => {
            // Handle nil (None) as empty for optional fields
            let _ = writeln!(out, "    expect({field_expr}.nil? || {field_expr}.empty?).to be(true)");
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(json_to_ruby).collect();
                let arr_str = items.join(", ");
                let _ = writeln!(
                    out,
                    "    expect([{arr_str}].any? {{ |v| {field_expr}.to_s.include?(v) }}).to be(true)"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                let _ = writeln!(out, "    expect({field_expr}).to be > {rb_val}");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                let _ = writeln!(out, "    expect({field_expr}).to be < {rb_val}");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                let _ = writeln!(out, "    expect({field_expr}).to be >= {rb_val}");
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                let _ = writeln!(out, "    expect({field_expr}).to be <= {rb_val}");
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let _ = writeln!(out, "    expect({field_expr}).to start_with({rb_val})");
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let _ = writeln!(out, "    expect({field_expr}).to end_with({rb_val})");
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    expect({field_expr}.length).to be >= {n}");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    expect({field_expr}.length).to be <= {n}");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    expect({field_expr}.length).to be >= {n}");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    expect({field_expr}.length).to eq({n})");
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "    expect({field_expr}).to be true");
        }
        "is_false" => {
            let _ = writeln!(out, "    expect({field_expr}).to be false");
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
                match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            if let Some(b) = val.as_bool() {
                                let _ = writeln!(out, "    expect({call_expr}).to be {b}");
                            } else {
                                let rb_val = json_to_ruby(val);
                                let _ = writeln!(out, "    expect({call_expr}).to eq({rb_val})");
                            }
                        }
                    }
                    "is_true" => {
                        let _ = writeln!(out, "    expect({call_expr}).to be true");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "    expect({call_expr}).to be false");
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            let _ = writeln!(out, "    expect({call_expr}).to be >= {rb_val}");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(out, "    expect({call_expr}.length).to be >= {n}");
                        }
                    }
                    "is_error" => {
                        let _ = writeln!(out, "    expect {{ {call_expr} }}.to raise_error");
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            let _ = writeln!(out, "    expect({call_expr}).to include({rb_val})");
                        }
                    }
                    other_check => {
                        panic!("Ruby e2e generator: unsupported method_result check type: {other_check}");
                    }
                }
            } else {
                panic!("Ruby e2e generator: method_result assertion missing 'method' field");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let _ = writeln!(out, "    expect({field_expr}).to match({rb_val})");
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
    let snake_method = method_name;
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

    setup_lines.push(format!("  def {snake_method}({params})"));
    match action {
        CallbackAction::Skip => {
            setup_lines.push("    'skip'".to_string());
        }
        CallbackAction::Continue => {
            setup_lines.push("    'continue'".to_string());
        }
        CallbackAction::PreserveHtml => {
            setup_lines.push("    'preserve_html'".to_string());
        }
        CallbackAction::Custom { output } => {
            let escaped = ruby_string_literal(output);
            setup_lines.push(format!("    {{ custom: {escaped} }}"));
        }
        CallbackAction::CustomTemplate { template } => {
            let escaped = ruby_string_literal(template);
            setup_lines.push(format!("    {{ custom: {escaped} }}"));
        }
    }
    setup_lines.push("  end".to_string());
}
