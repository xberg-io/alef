//! Ruby e2e test generator using RSpec.
//!
//! Generates `e2e/ruby/Gemfile` and `spec/{category}_spec.rb` files from
//! JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;
use crate::core::version::to_rubygems_prerelease;
use crate::e2e::codegen::resolve_field;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{escape_ruby_single, ruby_string_literal, sanitize_filename, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{
    Assertion, CallbackAction, Fixture, FixtureGroup, TemplateReturnForm, ValidationErrorExpectation,
};
use anyhow::Result;
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};
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
        type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
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

        // Check if there are HTTP fixtures that need server-pattern harness
        let has_http_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| f.http.is_some());
        let uses_harness = has_http_fixtures && !e2e_config.harness.imports.is_empty();

        // Emit app_harness.rb when using server-pattern
        if uses_harness {
            files.push(GeneratedFile {
                path: output_base.join("app_harness.rb"),
                content: render_app_harness(e2e_config, groups),
                generated_header: true,
            });
        }

        // Check if any fixture is an HTTP test (needs mock server bootstrap).
        let has_mock_server_fixtures = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| f.needs_mock_server());

        // Check if any fixture uses file_path or bytes args (needs chdir to test_documents).
        let has_file_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
            let cc = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            cc.args
                .iter()
                .any(|a| a.arg_type == "file_path" || a.arg_type == "bytes")
        });

        // Always generate spec/spec_helper.rb when file-based, HTTP, or server-pattern fixtures are present.
        if has_file_fixtures || has_mock_server_fixtures || uses_harness {
            files.push(GeneratedFile {
                path: output_base.join("spec").join("spec_helper.rb"),
                content: render_spec_helper(
                    has_file_fixtures,
                    has_mock_server_fixtures,
                    uses_harness,
                    &e2e_config.test_documents_relative_from(1),
                    &gem_name,
                    &module_path,
                    &e2e_config.harness.host,
                    e2e_config.harness.port,
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

            // Skip the entire file if no fixture in this category produces output.
            let has_any_output = active.iter().any(|f| {
                // HTTP tests always produce output.
                if f.is_http_test() {
                    return true;
                }
                let cc = e2e_config.resolve_call_for_fixture(
                    f.call.as_deref(),
                    &f.id,
                    &f.resolved_category(),
                    &f.tags,
                    &f.input,
                );
                let fr = FieldResolver::new(
                    e2e_config.effective_fields(cc),
                    e2e_config.effective_fields_optional(cc),
                    e2e_config.effective_result_fields(cc),
                    e2e_config.effective_fields_array(cc),
                    &std::collections::HashSet::new(),
                );
                let expects_error = f.assertions.iter().any(|a| a.assertion_type == "error");
                let has_not_error = f.assertions.iter().any(|a| a.assertion_type == "not_error");
                expects_error || has_not_error || has_usable_assertion(f, &fr, result_is_simple)
            });
            if !has_any_output {
                continue;
            }

            let filename = format!("{}_spec.rb", sanitize_filename(&group.category));
            let content = render_spec_file(
                &group.category,
                &active,
                &module_path,
                class_name.as_deref(),
                &gem_name,
                options_type.as_deref(),
                enum_fields,
                result_is_simple,
                e2e_config,
                has_file_fixtures || has_mock_server_fixtures,
                uses_harness,
                &config.adapters,
                config,
                type_defs,
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

fn render_app_harness(e2e_config: &E2eConfig, groups: &[FixtureGroup]) -> String {
    // Collect all HTTP fixtures from all groups.
    let mut fixtures_map = serde_json::Map::new();

    for group in groups {
        for fixture in &group.fixtures {
            if fixture.http.is_none() {
                continue;
            }
            // Convert the fixture to JSON for the harness to load.
            let http_data = &fixture.http.as_ref().unwrap();
            let fixture_json = serde_json::json!({
                "http": {
                    "handler": {
                        "route": &http_data.handler.route,
                        "method": &http_data.handler.method,
                        "body_schema": http_data.handler.body_schema.clone(),
                    },
                    "request": {
                        "path": &http_data.request.path,
                    },
                    "expected_response": {
                        "status_code": http_data.expected_response.status_code,
                        "body": &http_data.expected_response.body,
                        "headers": &http_data.expected_response.headers,
                    }
                }
            });
            fixtures_map.insert(fixture.id.clone(), fixture_json);
        }
    }

    let fixtures_json_raw = serde_json::to_string(&fixtures_map).unwrap_or_default();
    // Escape the JSON for safe embedding in a Ruby string literal
    let fixtures_json = ruby_string_literal(&fixtures_json_raw);

    // Apply language-specific overrides for Ruby
    let imports = e2e_config
        .harness
        .imports_for_lang("ruby")
        .into_iter()
        .collect::<Vec<_>>();
    let imports_ref = if !imports.is_empty() {
        &imports
    } else {
        &e2e_config.harness.imports
    };

    let app_class_override = e2e_config.harness.app_class_for_lang("ruby");
    let app_class_str = if let Some(ref ac) = app_class_override {
        ac.as_str()
    } else if let Some(ref ac) = e2e_config.harness.app_class {
        ac.as_str()
    } else {
        ""
    };

    // Ruby method names are snake_case by convention. `register_method_idiomatic`
    // preserves snake_case verbatim for ruby and applies any per-language override.
    let register_route_method = e2e_config
        .harness
        .register_method_idiomatic("ruby")
        .unwrap_or_else(|| "register_route".to_string());
    let register_route_method_str = register_route_method.as_str();

    let body_schema_setter = &e2e_config.harness.body_schema_setter;
    let method_enum = &e2e_config.harness.method_enum;

    let run_method_override = e2e_config.harness.run_method_for_lang("ruby");
    let run_method_str = if let Some(ref rm) = run_method_override {
        rm.as_str()
    } else if let Some(ref rm) = e2e_config.harness.run_method {
        rm.as_str()
    } else {
        "run"
    };
    let host = &e2e_config.harness.host;
    let port = e2e_config.harness.port;

    let header = hash::header(CommentStyle::Hash);

    // Derive Ruby-namespaced class names from imports[0] when explicit values are not configured.
    // E.g. imports[0] = "my_pkg" → module prefix "MyPkg::" → "MyPkg::Method", "MyPkg::App", etc.
    let module_prefix = if !imports_ref.is_empty() {
        format!("{}::", ruby_module_name(&imports_ref[0]))
    } else {
        String::new()
    };
    let method_enum_module = method_enum
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{module_prefix}Method"));
    let derived_app_class = format!("{module_prefix}App");
    let derived_route_builder_class = format!("{module_prefix}RouteBuilder");
    let derived_server_config_class = format!("{module_prefix}ServerConfig");

    let ctx = minijinja::context! {
        header => header,
        imports => imports_ref,
        app_class => if !app_class_str.is_empty() { app_class_str } else { derived_app_class.as_str() },
        route_builder_class => derived_route_builder_class.as_str(),
        server_config_class => derived_server_config_class.as_str(),
        route_builder_schema_setter => body_schema_setter.as_deref().unwrap_or("request_schema_json"),
        method_enum_module => method_enum_module,
        register_route_method => register_route_method_str,
        run_method => run_method_str,
        response_body_field => e2e_config.harness.response_body_field.as_str(),
        host => host,
        port => port,
        fixtures_json => fixtures_json,
    };

    crate::e2e::template_env::render("ruby/app_harness.rb.jinja", ctx)
}

fn render_gemfile(
    gem_name: &str,
    gem_path: &str,
    gem_version: &str,
    dep_mode: crate::e2e::config::DependencyMode,
) -> String {
    let gem_line = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            // If alef.toml provides the version with a rubygems operator (`~>`, `>=`,
            // `==`, etc.), the caller has chosen the registry-conventional form already
            // — use it verbatim. Otherwise apply the rubygems pre-release renderer and
            // wrap with `~> `.
            let trimmed = gem_version.trim_start();
            let constraint = if trimmed.starts_with(['~', '>', '<', '=', '!']) {
                gem_version.to_string()
            } else {
                format!("~> {}", to_rubygems_prerelease(gem_version))
            };
            format!("gem '{gem_name}', '{constraint}'")
        }
        crate::e2e::config::DependencyMode::Local => format!("gem '{gem_name}', path: '{gem_path}'"),
    };
    crate::e2e::template_env::render(
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

#[allow(clippy::too_many_arguments)]
fn render_spec_helper(
    has_file_fixtures: bool,
    has_mock_server_fixtures: bool,
    uses_harness: bool,
    test_documents_path: &str,
    _gem_name: &str,
    module_path: &str,
    harness_host: &str,
    _harness_port: u16,
) -> String {
    let header = hash::header(CommentStyle::Hash);
    let mut out = header;
    out.push_str("# frozen_string_literal: true\n");
    let _module_name = ruby_module_name(module_path);

    // Note: spec_helper.rb may contain library-specific registry cleanup hooks
    // (e.g., tracking plugin backends, clearing test-prefixed stubs between tests).
    // These are left for the consuming library to add—alef spec_helper is generic
    // and includes only universal setup patterns (file paths, mock servers, harness).

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

    if uses_harness {
        let _ = writeln!(out);
        let _ = writeln!(out, "require 'socket'");
        let _ = writeln!(out, "require 'open3'");
        let _ = writeln!(out, "require 'timeout'");
        let _ = writeln!(out);
        let harness_setup = format!(
            r#"# Spawn the app harness for server-pattern e2e tests.
# If SUT_URL is already set, a parent process started a shared harness.
# Use it as-is and do NOT spawn our own.
RSpec.configure do |config|
  config.before(:suite) do
    next if ENV['SUT_URL'] && !ENV['SUT_URL'].empty?
    harness_bin = File.expand_path('../app_harness.rb', __dir__)
    unless File.exist?(harness_bin)
      raise "app_harness.rb not found at #{{harness_bin}}"
    end
    # Spawn the harness and read its stdout to extract the dynamic port.
    @_harness_stdin, @_harness_stdout, @_harness_stderr, @_harness_thread = Open3.popen3('ruby', harness_bin)
    @_harness_pid = @_harness_thread.pid
    harness_port = nil
    deadline = Time.now + 15.0
    # Read stdout, collecting all HARNESS_PORT lines. The harness retries on bind
    # failure, so we may see multiple ports. Keep the latest one and verify it's reachable.
    latest_port = nil
    while Time.now < deadline
      if @_harness_thread.status.nil?
        # Process died; use the latest port if available
        harness_port = latest_port if latest_port
        break
      end
      begin
        Timeout.timeout(0.1) do
          line = @_harness_stdout.readline
          if line =~ /^HARNESS_PORT=(\d+)/
            latest_port = $1.to_i
          end
        end
      rescue Timeout::Error, EOFError, Errno::EAGAIN
        # Try to verify the latest port if we have one
        if latest_port
          begin
            TCPSocket.new('{}', latest_port).close
            harness_port = latest_port
            break  # Success: port is reachable
          rescue Errno::ECONNREFUSED, Errno::EHOSTUNREACH
            # Port not yet listening; keep polling
            sleep(0.05)
          end
        else
          sleep(0.05)
        end
      end
    end
    unless harness_port
      Process.kill('TERM', @_harness_pid) rescue nil
      msg = latest_port ? "App harness did not become reachable on {}:#{{latest_port}} within 15s" : "App harness did not report port within 15s"
      raise msg
    end
    url = "http://{}:#{{harness_port}}"
    ENV['SUT_URL'] = url
  end

  config.after(:suite) do
    if @_harness_pid
      Process.kill('TERM', @_harness_pid) rescue nil
      Process.wait(@_harness_pid, 5) rescue nil
    end
  end
end
"#,
            harness_host, harness_host, harness_host
        );
        out.push_str(&harness_setup);
    } else if has_mock_server_fixtures {
        out.push_str(
            r#"
require 'json'
require 'open3'

# Spawn the mock-server binary and set MOCK_SERVER_URL for all tests.
#
# Two execution modes:
# 1. External mode (`alef test-apps run` parent): MOCK_SERVER_URL is already set.
#    Use it as-is together with any MOCK_SERVERS / MOCK_SERVER_<FIXTURE_ID> vars
#    that the parent exported. Do NOT spawn our own server.
# 2. Standalone mode (direct `bundle exec rspec` / `task ruby:smoke`): Build the
#    mock-server binary if it is missing, then spawn it, capture its URL, and
#    tear it down on exit.
RSpec.configure do |config|
  config.before(:suite) do
    next if ENV['MOCK_SERVER_URL'] && !ENV['MOCK_SERVER_URL'].empty?
    bin = File.expand_path('../../rust/target/release/mock-server', __dir__)
    fixtures_dir = File.expand_path('../../../fixtures', __dir__)
    unless File.exist?(bin)
      # Build the mock-server from the e2e/rust/ crate that alef generated.
      manifest = File.expand_path('../../rust/Cargo.toml', __dir__)
      raise "mock-server Cargo.toml not found at #{manifest}" unless File.exist?(manifest)
      system(
        'cargo', 'build', '--release',
        '--manifest-path', manifest,
        '--bin', 'mock-server',
        exception: true
      )
      raise "mock-server binary still missing after build: #{bin}" unless File.exist?(bin)
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
    crate::e2e::template_env::render("ruby/rubocop.yml.jinja", minijinja::context! {})
}

#[allow(clippy::too_many_arguments)]
fn render_spec_file(
    category: &str,
    fixtures: &[&Fixture],
    module_path: &str,
    class_name: Option<&str>,
    gem_name: &str,
    options_type: Option<&str>,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    needs_spec_helper: bool,
    uses_harness: bool,
    adapters: &[crate::core::config::extras::AdapterConfig],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
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
    let ruby_module = ruby_module_name(module_path);
    let call_receiver = class_name.map(|s| s.to_string()).unwrap_or_else(|| ruby_module.clone());

    // Check for array contains assertions
    let has_array_contains = fixtures.iter().any(|fixture| {
        let cc = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        let fr = FieldResolver::new(
            e2e_config.effective_fields(cc),
            e2e_config.effective_fields_optional(cc),
            e2e_config.effective_result_fields(cc),
            e2e_config.effective_fields_array(cc),
            &std::collections::HashSet::new(),
        );
        fixture.assertions.iter().any(|a| {
            matches!(a.assertion_type.as_str(), "contains" | "contains_all" | "not_contains")
                && a.field
                    .as_deref()
                    .is_some_and(|f| !f.is_empty() && fr.is_array(fr.resolve(f)))
        })
    });

    // Build examples
    let mut examples = Vec::new();
    for fixture in fixtures {
        if fixture.http.is_some() {
            // HTTP example is handled separately (uses shared driver or server-pattern)
            let mut out = String::new();
            if uses_harness {
                render_http_example_sut(&mut out, fixture);
            } else {
                render_http_example(&mut out, fixture);
            }
            examples.push(out);
        } else {
            // Resolve per-fixture call config so we can detect streaming up front.
            let fixture_call = e2e_config.resolve_call_for_fixture(
                fixture.call.as_deref(),
                &fixture.id,
                &fixture.resolved_category(),
                &fixture.tags,
                &fixture.input,
            );
            // Build per-call field resolver using the effective field sets for this call.
            let fixture_call_resolver = FieldResolver::new(
                e2e_config.effective_fields(fixture_call),
                e2e_config.effective_fields_optional(fixture_call),
                e2e_config.effective_result_fields(fixture_call),
                e2e_config.effective_fields_array(fixture_call),
                &std::collections::HashSet::new(),
            );
            let field_resolver = &fixture_call_resolver;
            let fixture_call_overrides = fixture_call.overrides.get("ruby");
            let raw_function_name = fixture_call_overrides
                .and_then(|o| o.function.as_ref())
                .cloned()
                .unwrap_or_else(|| fixture_call.function.clone());

            let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
            let has_not_error = fixture.assertions.iter().any(|a| a.assertion_type == "not_error");
            let has_usable = has_usable_assertion(fixture, field_resolver, result_is_simple);
            let is_streaming =
                super::streaming_assertions::resolve_is_streaming(fixture, fixture_call.streaming_enabled());

            // Ruby has FFI access to the Rust core, so it can execute non-HTTP
            // fixtures. Render tests for all fixtures that have error assertions,
            // not_error assertions, streaming calls, or are explicitly testable.
            // Fixtures with no assertions remain skipped as genuinely untestable.
            if !expects_error && !has_usable && !has_not_error && !is_streaming && fixture.assertions.is_empty() {
                let test_name = sanitize_ident(&fixture.id);
                let description_literal = ruby_string_literal(&format!("{test_name}: {}", fixture.description));
                let mut out = String::new();
                out.push_str(&format!("  it {description_literal} do\n"));
                out.push_str("    skip 'Fixture has no assertions to validate'\n");
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
                // Use fixture.resolved_args() so per-fixture args (e.g. trait-bridge
                // test_backend stubs) take precedence over the call-config default.
                let fixture_args = fixture.resolved_args(fixture_call);
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
                let adapter_req_type_owned: Option<String> = adapters
                    .iter()
                    .find(|a| a.name == fixture_call.function.as_str())
                    .and_then(|a| a.request_type.as_deref())
                    .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());
                let streaming_item_type_owned = crate::e2e::codegen::recipe::streaming_item_type(
                    fixture_call,
                    adapters,
                    &[fixture_call.function.as_str()],
                )
                .map(str::to_string);
                let example = if is_streaming {
                    render_chat_stream_example(
                        fixture,
                        &fixture_function_name,
                        &call_receiver,
                        &ruby_module,
                        fixture_args,
                        fixture_options_type,
                        fixture_enum_fields,
                        e2e_config,
                        fixture_client_factory,
                        &fixture_extra_args,
                        adapter_req_type_owned.as_deref(),
                        streaming_item_type_owned.as_deref(),
                        config,
                        type_defs,
                    )
                } else {
                    render_example(
                        fixture,
                        &fixture_function_name,
                        &call_receiver,
                        &ruby_module,
                        fixture_result_var,
                        fixture_args,
                        field_resolver,
                        fixture_options_type,
                        fixture_enum_fields,
                        e2e_config.effective_fields_enum(fixture_call),
                        fixture_result_is_simple,
                        fixture_call.returns_void,
                        e2e_config,
                        fixture_client_factory,
                        &fixture_extra_args,
                        adapter_req_type_owned.as_deref(),
                        config,
                        type_defs,
                    )
                };
                examples.push(example);
            }
        }
    }

    let header = hash::header(CommentStyle::Hash);
    crate::e2e::template_env::render(
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
        let description_literal = ruby_string_literal(description);
        let rendered = crate::e2e::template_env::render(
            "ruby/http_test.jinja",
            minijinja::context! {
                fn_name => fn_name,
                description => description_literal,
                skip_reason => skip_reason,
            },
        );
        out.push_str(&rendered);
    }

    /// Close the inner `it` block and the outer `describe` block.
    fn render_test_close(&self, out: &mut String) {
        let rendered = crate::e2e::template_env::render("ruby/http_test_close.jinja", minijinja::context! {});
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

        let rendered = crate::e2e::template_env::render(
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
            let description_literal = ruby_string_literal(&fixture.description);
            let method = http.request.method.to_uppercase();
            let path = &http.request.path;
            let rendered = crate::e2e::template_env::render(
                "ruby/http_101_skip.jinja",
                minijinja::context! {
                    method => method,
                    path => path,
                    description => description_literal,
                },
            );
            out.push_str(&rendered);
        }
        return;
    }

    client::http_call::render_http_test(out, &RubyTestClientRenderer, fixture);
}

/// Render an RSpec example for an HTTP server-pattern test fixture (SUT harness).
///
/// Uses the server-pattern template to hit the actual SUT harness listening on
/// a configured host:port, rather than the shared mock-server driver.
fn render_http_example_sut(out: &mut String, fixture: &Fixture) {
    let Some(http) = &fixture.http else {
        return;
    };

    // HTTP 101 (WebSocket upgrade) cannot be tested via Net::HTTP.
    if http.expected_response.status_code == 101 {
        let description_literal = ruby_string_literal(&fixture.description);
        let method = http.request.method.to_uppercase();
        let path = &http.request.path;
        let rendered = crate::e2e::template_env::render(
            "ruby/http_101_skip.jinja",
            minijinja::context! {
                method => method,
                path => path,
                description => description_literal,
            },
        );
        out.push_str(&rendered);
        return;
    }

    let fn_name = sanitize_ident(&fixture.id);
    let description = &fixture.description;
    let desc_with_period = if description.ends_with('.') {
        description.to_string()
    } else {
        format!("{description}.")
    };
    let description_literal = ruby_string_literal(&desc_with_period);

    // Build request headers dict literal
    let mut header_entries: Vec<String> = http
        .request
        .headers
        .iter()
        .map(|(k, v)| format!("      '{}' => '{}',", k, v))
        .collect();
    header_entries.sort();
    let headers_ruby = if header_entries.is_empty() {
        "{}".to_string()
    } else {
        format!("{{\n{}\n    }}", header_entries.join("\n"))
    };

    let method = http.request.method.to_uppercase();
    let method_class = http_method_class(&method);
    let path = format!("/fixtures/{}{}", &fixture.id, &http.request.path);

    // Determine request body.
    // When the fixture body is a JSON string (e.g. URL-encoded form data like
    // "a=1&b=2"), it must be sent as a raw string, NOT wrapped in JSON.dump().
    // Detect this by checking whether the body JSON value is a string.
    let (has_body, body_ruby, is_raw_body) = if let Some(body) = &http.request.body {
        let is_raw = body.is_string();
        (true, json_to_ruby(body), is_raw)
    } else {
        (false, String::new(), false)
    };

    // Determine response body expectations
    let (has_text_body, text_ruby) = if let Some(serde_json::Value::String(s)) = &http.expected_response.body {
        (true, ruby_string_literal(s))
    } else {
        (false, String::new())
    };

    let (has_json_body, json_ruby) = if let Some(body) = &http.expected_response.body {
        if !(body.is_null() || body.is_string() && body.as_str() == Some("")) {
            if !matches!(body, serde_json::Value::String(_)) {
                (true, json_to_ruby(body))
            } else {
                (false, String::new())
            }
        } else {
            (false, String::new())
        }
    } else {
        (false, String::new())
    };

    let (has_partial_body, partial_body_checks) = if let Some(partial) = &http.expected_response.body_partial {
        if let Some(obj) = partial.as_object() {
            let checks: Vec<minijinja::Value> = obj
                .iter()
                .map(|(key, val)| {
                    let ruby_val = json_to_ruby(val);
                    minijinja::context! {
                        key => key,
                        value => ruby_val,
                    }
                })
                .collect();
            (true, checks)
        } else {
            (false, Vec::new())
        }
    } else {
        (false, Vec::new())
    };

    // Build header assertions
    let mut header_assertions: Vec<minijinja::Value> = Vec::new();
    let mut header_names: Vec<String> = http.expected_response.headers.keys().cloned().collect();
    header_names.sort();

    for name in header_names {
        let value = &http.expected_response.headers[&name];
        header_assertions.push(minijinja::context! {
            name => name,
            assertion_type => "eq",
            value => value,
        });
    }

    // Build validation error expectations
    let (has_validation_errors, validation_errors) = if http.expected_response.status_code == 422 {
        if let Some(body) = &http.expected_response.body {
            if let Some(obj) = body.as_object() {
                if let Some(errs) = obj.get("errors").and_then(|v| v.as_array()) {
                    let ve: Vec<minijinja::Value> = errs
                        .iter()
                        .filter_map(|err| {
                            let loc = err.get("loc").and_then(|l| l.as_array())?;
                            let msg = err.get("msg").and_then(|m| m.as_str())?;
                            // Produce comma-separated element literals so the template can
                            // wrap them in `[...]` to form a valid Ruby array literal.
                            // e.g. loc = ["query", "limit"] → loc_ruby = "'query', 'limit'"
                            // Template: `[{{ loc_ruby }}]` → `['query', 'limit']`
                            let loc_ruby = loc.iter().map(json_to_ruby).collect::<Vec<_>>().join(", ");
                            // Escape single quotes for embedding in a Ruby single-quoted string.
                            // `ruby_string_literal` would choose double-quotes, but the template
                            // embeds the value directly inside `'...'`, so we must escape `'` → `\'`.
                            let escaped = escape_ruby_single(msg);
                            Some(minijinja::context! {
                                loc_ruby => loc_ruby,
                                escaped_msg => escaped,
                            })
                        })
                        .collect();
                    (true, ve)
                } else {
                    (false, Vec::new())
                }
            } else {
                (false, Vec::new())
            }
        } else {
            (false, Vec::new())
        }
    } else {
        (false, Vec::new())
    };

    let rendered = crate::e2e::template_env::render(
        "ruby/http_test_sut.jinja",
        minijinja::context! {
            fn_name => fn_name,
            description => description_literal,
            method => method,
            method_class => method_class,
            path => path,
            headers_ruby => headers_ruby,
            has_body => has_body,
            body_ruby => body_ruby,
            is_raw_body => is_raw_body,
            expected_status => http.expected_response.status_code,
            has_text_body => has_text_body,
            text_ruby => text_ruby,
            has_json_body => has_json_body,
            json_ruby => json_ruby,
            has_partial_body => has_partial_body,
            partial_body_checks => partial_body_checks,
            header_assertions => header_assertions,
            has_validation_errors => has_validation_errors,
            validation_errors => validation_errors,
        },
    );
    out.push_str(&rendered);
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
/// generated stream chunk. The codegen builds local aggregator vars
/// (`chunks`, `stream_content`, `stream_complete`, plus optional
/// `last_finish_reason`, `tool_calls_json`, `total_tokens`) inside the block and
/// then emits assertions on those locals — never on response pseudo-fields.
#[allow(clippy::too_many_arguments)]
fn render_chat_stream_example(
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
fn emit_chat_stream_assertion(
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

// ---------------------------------------------------------------------------
// Function-call test rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_example(
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

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// Returns `(setup_lines, args_string)`.
/// Emit Ruby object-array fixture values for a typed `json_object` array.
fn emit_ruby_object_array(arr: &serde_json::Value) -> String {
    if let Some(items) = arr.as_array() {
        let item_strs: Vec<String> = items
            .iter()
            .filter_map(|item| {
                item.as_object()
                    .map(|obj| json_to_ruby(&serde_json::Value::Object(obj.clone())))
            })
            .collect();
        format!("[{}]", item_strs.join(", "))
    } else {
        "[]".to_string()
    }
}

#[allow(clippy::too_many_arguments)]
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    call_receiver: &str,
    module_name: &str,
    options_type: Option<&str>,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    fixture: &crate::e2e::fixture::Fixture,
    adapter_request_type: Option<&str>,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> (Vec<String>, String, Vec<String>) {
    let fixture_id = &fixture.id;
    if args.is_empty() {
        // No args config: don't pass the input as a function argument.
        // The input data is for setup/mocking purposes only. Functions with no
        // parameters must be called with no arguments — not with `{}` or `nil`.
        return (Vec::new(), String::new(), Vec::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    // Teardown lines emitted after the call+assertions. Populated by
    // trait-bridge args so RSpec's shared-process registry state is restored
    // between tests (e.g. `<Binding>.unregister_<trait>('test-backend')`).
    let mut teardown_lines: Vec<String> = Vec::new();
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
            if let Some(req_type) = adapter_request_type {
                let req_var = format!("{}_req", arg.name);
                // Derive the module qualifier from module_name (e.g. "DemoCrawler")
                let mod_qualifier = ruby_module_name(module_name);
                setup_lines.push(format!(
                    "{req_var} = {mod_qualifier}::{req_type}.new(url: {})",
                    arg.name
                ));
                parts.push(req_var);
            } else {
                parts.push(arg.name.clone());
            }
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            // Array of URLs: each element is either a bare path (`/seed1`) — prefixed
            // with the per-fixture mock-server URL at runtime — or an absolute URL kept
            // as-is. Mirrors `mock_url` resolution: `MOCK_SERVER_<FIXTURE_ID>` first,
            // then `MOCK_SERVER_URL/fixtures/<id>`. Without this branch the codegen
            // falls back to a JSON-array literal of bare relative paths and the Rust
            // HTTP client rejects them.
            // Flush any pending nil placeholders before this positional arg.
            for _ in 0..skipped_optional_count {
                parts.push("nil".to_string());
            }
            skipped_optional_count = 0;
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field).unwrap_or(&serde_json::Value::Null);
            let paths: Vec<String> = if let Some(arr) = val.as_array() {
                arr.iter().filter_map(|v| v.as_str().map(ruby_string_literal)).collect()
            } else {
                Vec::new()
            };
            let paths_literal = paths.join(", ");
            let name = &arg.name;
            setup_lines.push(format!(
                "{name}_base = ENV.fetch('{env_key}', nil) || \"#{{ENV.fetch('MOCK_SERVER_URL')}}/fixtures/{fixture_id}\""
            ));
            setup_lines.push(format!(
                "{name} = [{paths_literal}].map {{ |p| p.start_with?('http') ? p : \"#{{{name}_base}}#{{p}}\" }}"
            ));
            parts.push(name.clone());
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

        if arg.arg_type == "test_backend" {
            // Flush any pending nil placeholders for skipped optionals before this positional arg.
            for _ in 0..skipped_optional_count {
                parts.push("nil".to_string());
            }
            skipped_optional_count = 0;
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| t.methods.iter().collect())
                        .unwrap_or_default();
                    let emission = crate::e2e::codegen::emit_test_backend("ruby", trait_bridge, &methods, fixture);
                    // Split multi-line setup_block into individual lines so the
                    // Jinja template can indent each line uniformly with `    {{ line }}`.
                    for line in emission.setup_block.lines() {
                        setup_lines.push(line.to_string());
                    }
                    parts.push(emission.arg_expr);

                    // For register_fn traits (plugin pattern), Magnus requires a second "name" argument.
                    // Extract the backend name from fixture input (same logic as emit_test_backend).
                    if trait_bridge.register_fn.is_some() {
                        let backend_name = extract_backend_name_from_input(&fixture.input, &fixture.id);
                        parts.push(ruby_string_literal(&backend_name));

                        // Emit `<module>.<unregister_fn>('<name>')` after the call so
                        // RSpec's single-process registry state is restored between
                        // tests. Without this, the next trait-using fixture fails
                        // because the test registry contains only the test
                        // stub and the core's `ensure_*_initialized` self-heal only
                        // triggers when the registry is empty.
                        if let Some(unregister_fn) = trait_bridge.unregister_fn.as_deref() {
                            teardown_lines.push(format!(
                                "{call_receiver}.{unregister_fn}({})",
                                ruby_string_literal(&backend_name)
                            ));
                        }
                    }
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("ruby");
            setup_lines.push(format!("# {}", emission.arg_expr));
            parts.push("nil".to_string());
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
                    // Check for typed object arrays (element_type set)
                    if let Some(_elem_type) = &arg.element_type {
                        if v.is_array() {
                            if let Some(arr) = v.as_array() {
                                // Only emit as tagged-enum array if all elements are objects.
                                // Otherwise fall through to json_to_ruby for primitive arrays (e.g., String, Int).
                                if !arr.is_empty() && arr.iter().all(|item| item.is_object()) {
                                    parts.push(emit_ruby_object_array(v));
                                    continue;
                                }
                            }
                            // Fall through if array is empty or contains non-objects (primitives)
                        }
                    }
                    // Otherwise handle regular options_type objects
                    if let (Some(opts_type), Some(obj)) = (options_type, v.as_object()) {
                        let kwargs: Vec<String> = obj
                            .iter()
                            .filter_map(|(k, vv)| {
                                // Skip empty string values (they cause enum parsing failures)
                                if let Some(s) = vv.as_str() {
                                    if s.is_empty() {
                                        return None; // Skip all empty strings
                                    }
                                    // For known enum fields, use snake_case enum variant
                                    if enum_fields.contains_key(k) {
                                        let snake_key = k.to_snake_case();
                                        let snake_val = s.to_snake_case();
                                        return Some(format!("{snake_key}: '{snake_val}'"));
                                    }
                                }
                                let snake_key = k.to_snake_case();
                                let rb_val = json_to_ruby(vv);
                                Some(format!("{snake_key}: {rb_val}"))
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

    (setup_lines, parts.join(", "), teardown_lines)
}

#[allow(clippy::too_many_arguments)]
fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    fields_enum: &HashSet<String>,
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
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            out.push_str(&format!("    expect({result_var}).to eq({rb_val})\n"));
                        }
                        return;
                    }
                    "contains" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = crate::e2e::escape::ruby_string_literal(s);
                            out.push_str(&format!("    expect({result_var}).to include({escaped})\n"));
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
        // Skip enum variant accessors (metadata.format.excel etc.) — Magnus serializes
        // FormatMetadata to JSON, so variants are unavailable in Ruby
        if f.contains("metadata.format.") && f.contains(".") {
            out.push_str(&format!(
                "    # skipped: enum variant accessor '{f}' not available on Ruby (serialized to Hash)\n"
            ));
            return;
        }

        // For metadata.format (enum, serialized to Hash), skip since the serialization
        // format differs between languages and doesn't preserve Display formatting
        if f == "metadata.format" {
            out.push_str("    # skipped: metadata.format enum field serialization differs in Ruby\n");
            return;
        }

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
            "chunks_have_heading_context" | "first_chunk_starts_with_heading" => {
                out.push_str(&format!(
                    "    # skipped: synthetic field '{f}' not available on Ruby Chunk binding\n"
                ));
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
            // Ruby ProcessingResult does not expose result_keywords; skip.
            "keywords" | "keywords_count" => {
                out.push_str(&format!(
                    "    # skipped: field '{f}' not available on Ruby ProcessingResult\n"
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
    // project config that already labels e.g. `status = "BatchStatus"` for the
    // Java/C#/Python sides should apply here too without a Ruby-only duplicate.
    let field_is_enum = assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        fields_enum.contains(f)
            || fields_enum.contains(resolved)
            || per_call_enum_fields.contains_key(f)
            || per_call_enum_fields.contains_key(resolved)
    });
    // For string equality on simple-result calls we want `.to_s.strip` to absorb
    // trailing whitespace, but for numeric/bool simple results that coercion turns
    // `0` into `"0"` and the `eq(0)` Integer comparison fails. Only fold `.to_s.strip`
    // into the simple-result path when the expected value is a string; otherwise
    // keep the raw expression so numeric/bool comparisons stay typed.
    let expected_is_string = assertion.value.as_ref().is_some_and(|v| v.is_string());
    let stripped_field_expr = if result_is_simple && expected_is_string {
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
                // Mirror Python's `expr.strip() == expected.strip()` pattern when comparing
                // string values: converters commonly emit a trailing newline that fixture
                // authors don't write into the expected string.
                let cmp_expr = if expected.is_string() && !field_is_enum {
                    format!("{stripped_field_expr}.to_s.strip")
                } else {
                    stripped_field_expr.clone()
                };
                let cmp_expected = if expected.is_string() && !field_is_enum {
                    format!("{rb_val}.strip")
                } else {
                    rb_val
                };

                let rendered = crate::e2e::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "equals",
                        stripped_field_expr => cmp_expr,
                        is_boolean_val => is_boolean_val,
                        bool_val => bool_val,
                        expected_val => cmp_expected,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let rendered = crate::e2e::template_env::render(
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
                let rendered = crate::e2e::template_env::render(
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
                let rendered = crate::e2e::template_env::render(
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
            let rendered = crate::e2e::template_env::render(
                "ruby/assertion.jinja",
                minijinja::context! {
                    assertion_type => "not_empty",
                    field_expr => field_expr.clone(),
                },
            );
            out.push_str(&rendered);
        }
        "is_empty" => {
            let rendered = crate::e2e::template_env::render(
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
                let rendered = crate::e2e::template_env::render(
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
                let rendered = crate::e2e::template_env::render(
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
                let rendered = crate::e2e::template_env::render(
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
                let rendered = crate::e2e::template_env::render(
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
                let rendered = crate::e2e::template_env::render(
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
                let rendered = crate::e2e::template_env::render(
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
                let rendered = crate::e2e::template_env::render(
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
                    let rendered = crate::e2e::template_env::render(
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
            let rendered = crate::e2e::template_env::render(
                "ruby/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_true",
                    field_expr => field_expr.clone(),
                },
            );
            out.push_str(&rendered);
        }
        "is_false" => {
            let rendered = crate::e2e::template_env::render(
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

                let rendered = crate::e2e::template_env::render(
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
                let rendered = crate::e2e::template_env::render(
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

/// Build a Ruby call expression for a `method_result` assertion on a sample_language Tree.
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

/// Convert a module path (e.g., "demo_markup") to Ruby PascalCase module name
/// (e.g., "DemoMarkup").
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
fn build_ruby_visitor(setup_lines: &mut Vec<String>, visitor_spec: &crate::e2e::fixture::VisitorSpec) -> String {
    setup_lines.push("visitor = Class.new do".to_string());
    for (method_name, action) in &visitor_spec.callbacks {
        emit_ruby_visitor_method(setup_lines, method_name, action);
    }
    setup_lines.push("end.new".to_string());
    "visitor".to_string()
}

/// Emit a Ruby visitor method for a callback action.
fn emit_ruby_visitor_method(setup_lines: &mut Vec<String>, method_name: &str, action: &CallbackAction) {
    let params = "*args";

    // Pre-compute action type and values
    let (action_type, action_value, return_form) = match action {
        CallbackAction::Skip => ("skip", String::new(), "dict"),
        CallbackAction::Continue => ("continue", String::new(), "dict"),
        CallbackAction::PreserveHtml => ("preserve_html", String::new(), "dict"),
        CallbackAction::Custom { output } => {
            let escaped = ruby_string_literal(output);
            ("custom", escaped, "dict")
        }
        CallbackAction::CustomTemplate { template, return_form } => {
            let interpolated = template.replace('\\', "\\\\").replace('"', "\\\"");
            let form = match return_form {
                TemplateReturnForm::Dict => "dict",
                TemplateReturnForm::BareString => "bare_string",
            };
            ("custom_template", format!("\"{interpolated}\""), form)
        }
    };

    let rendered = crate::e2e::template_env::render(
        "ruby/visitor_method.jinja",
        minijinja::context! {
            method_name => method_name,
            params => params,
            action_type => action_type,
            action_value => action_value,
            return_form => return_form,
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

/// Extract the canonical backend name from fixture input JSON.
///
/// Mirrors the lookup strategy used by the Python, PHP, and Rust e2e emitters.
/// Searches `input.name`, then any nested object's `name` field, then falls
/// back to `fixture_id`.
fn extract_backend_name_from_input(input: &serde_json::Value, fallback: &str) -> String {
    if let Some(obj) = input.as_object() {
        if let Some(s) = obj.get("name").and_then(|v| v.as_str()) {
            return s.to_string();
        }
        for v in obj.values() {
            if let Some(inner) = v.as_object() {
                if let Some(s) = inner.get("name").and_then(|v| v.as_str()) {
                    return s.to_string();
                }
            }
        }
        for v in obj.values() {
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
        }
    }
    fallback.to_string()
}

/// Emit a Ruby test backend stub.
///
/// Ruby is duck-typed: define an anonymous class that responds to each required method
/// and return a sensible default value. The Plugin super-trait `name` method returns the
/// backend name extracted from `fixture.input`. All other methods return their
/// language-native defaults. Named return types return `'{}'` so the Magnus bridge can
/// deserialise the return value via JSON.
///
/// The returned `setup_block` defines a local variable `stub_<id>` holding the
/// anonymous class instance. The `arg_expr` is the variable name; callers emit
/// `<Module>.<register_fn>(arg_expr, "<fixture_id>")`.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use crate::core::ir::{PrimitiveType, TypeRef};

    let defaults = language_defaults("ruby");
    let safe_id = sanitize_ident(&fixture.id);
    let backend_name = extract_backend_name_from_input(&fixture.input, &fixture.id);
    let var_name = format!("stub_{safe_id}");

    let mut setup = String::new();
    let _ = writeln!(setup, "{var_name} = Class.new do");

    // Plugin super-trait: emit unconditional super-trait methods.
    // The Magnus bridge calls these on every registered plugin object regardless of
    // whether Rust has a default implementation, so stubs must define them.
    if trait_bridge.super_trait.is_some() {
        let _ = writeln!(setup, "  def name = '{backend_name}'");
        let _ = writeln!(setup, "  def initialize");
        let _ = writeln!(setup, "    nil");
        let _ = writeln!(setup, "  end");
        let _ = writeln!(setup, "  def shutdown");
        let _ = writeln!(setup, "    nil");
        let _ = writeln!(setup, "  end");
        let _ = writeln!(setup, "  def version = '1.0.0'");
    }

    // Emit stubs for all required methods (skip those with default implementations).
    for method in methods.iter().filter(|m| !m.has_default_impl) {
        let ruby_name = method.name.to_snake_case();
        // Build a parameter list: positional param names only (Ruby is duck-typed).
        let params: Vec<String> = method.params.iter().map(|p| sanitize_ident(&p.name)).collect();
        let param_str = params.join(", ");
        // Named types are not defined in the Ruby binding scope.  The Magnus bridge
        // tries String#to_s then falls back to .to_json, so return a JSON-safe empty
        // object string '{}'  that round-trips through serde_json.
        //
        // For numeric types in test backends, use a nonzero integer default.
        let default_val = match &method.return_type {
            TypeRef::Named(_) => "'{}'".to_string(),
            TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "1".to_string(),
            other => defaults.emit_default(other),
        };
        if param_str.is_empty() {
            let _ = writeln!(setup, "  def {ruby_name} = {default_val}");
        } else {
            let _ = writeln!(setup, "  def {ruby_name}({param_str}) = {default_val}");
        }
    }

    let _ = writeln!(setup, "end.new");

    super::TestBackendEmission {
        setup_block: setup,
        arg_expr: var_name,
        type_imports: Vec::new(),
        teardown_block: String::new(),
    }
}

#[cfg(test)]
mod trait_bridge_tests {
    use super::{emit_test_backend, render_spec_helper};
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, ParamDef, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn make_fixture(id: &str) -> Fixture {
        Fixture {
            id: id.to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            assertions: vec![],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        }
    }

    fn make_param(name: &str, ty: TypeRef) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
        }
    }

    fn make_method(name: &str, params: Vec<(&str, TypeRef)>, ret: TypeRef, is_async: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: params.into_iter().map(|(n, ty)| make_param(n, ty)).collect(),
            return_type: ret,
            is_async,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    #[test]
    fn spec_helper_stays_generic_for_library_specific_setup() {
        let content = render_spec_helper(
            true,
            false,
            false,
            "../../fixtures",
            "custom_gem",
            "custom_module",
            "127.0.0.1",
            8000,
        );

        assert!(
            !content.contains("require 'custom_gem'"),
            "spec helper must not require the generated gem directly:\n{content}"
        );
        assert!(
            !content.contains("CustomModule") && !content.contains("SampleCrate") && !content.contains("sample_crate"),
            "spec helper must avoid library-specific module cleanup:\n{content}"
        );
    }

    /// Genericity test: a synthetic TestTrait with one sync method and Plugin super-trait
    /// must not reference any sample_core-domain names in setup_block or arg_expr.
    #[test]
    fn test_backend_emission_is_generic() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("SomeSuperTrait".to_string()),
            register_fn: Some("register_test_trait".to_string()),
            ..TraitBridgeConfig::default()
        };

        let do_thing = make_method(
            "do_thing",
            vec![("x", TypeRef::Primitive(crate::core::ir::PrimitiveType::I32))],
            TypeRef::String,
            false,
        );

        let fixture = make_fixture("my_test_fixture");
        let methods = vec![&do_thing];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        // setup_block must not reference any sample_core-domain trait or method names.
        assert!(
            !emission.setup_block.contains("OcrBackend"),
            "setup_block must not hardcode domain trait names, got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("process_image"),
            "setup_block must not hardcode domain method names, got:\n{}",
            emission.setup_block
        );
        // Must emit the method name from MethodDef.
        assert!(
            emission.setup_block.contains("do_thing"),
            "setup_block must contain the method name 'do_thing', got:\n{}",
            emission.setup_block
        );
        // Must emit Plugin name method when super_trait is set.
        assert!(
            emission.setup_block.contains("name"),
            "setup_block must emit 'name' for super_trait, got:\n{}",
            emission.setup_block
        );
        // arg_expr must reference the fixture id.
        assert!(
            emission.arg_expr.contains("my_test_fixture"),
            "arg_expr must reference fixture id, got: {}",
            emission.arg_expr
        );
    }

    /// Named return types must emit `'{}'` (JSON-safe string), not `TypeName.new`
    /// which would reference an undefined Ruby constant.
    #[test]
    fn test_backend_named_return_emits_json_string() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        let extract_bytes = make_method(
            "extract_bytes",
            vec![("content", TypeRef::Bytes), ("mime_type", TypeRef::String)],
            TypeRef::Named("HiddenRecord".to_string()),
            false,
        );

        let fixture = make_fixture("register_document_extractor_trait_bridge");
        let methods = vec![&extract_bytes];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("'{}'"),
            "Named return type must emit '{{}}' not a constructor call, got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("HiddenRecord.new"),
            "setup_block must not reference undefined constant HiddenRecord, got:\n{}",
            emission.setup_block
        );
    }

    /// Backend name must be extracted from fixture.input, not fixture.id.
    #[test]
    fn test_backend_name_from_input() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        let extract_bytes = make_method(
            "extract_bytes",
            vec![("content", TypeRef::Bytes)],
            TypeRef::Named("HiddenRecord".to_string()),
            false,
        );

        let mut fixture = make_fixture("register_document_extractor_trait_bridge");
        fixture.input = serde_json::json!({
            "extractor": { "type": "test", "name": "test-extractor" }
        });

        let methods = vec![&extract_bytes];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("test-extractor"),
            "setup_block must use input-derived name 'test-extractor', got:\n{}",
            emission.setup_block
        );
        // The fixture id appears in the variable name (stub_register_...) but
        // the name() method must return the input-derived name, not the fixture id.
        assert!(
            !emission
                .setup_block
                .contains("= 'register_document_extractor_trait_bridge'"),
            "name() method must not return fixture id, got:\n{}",
            emission.setup_block
        );
    }

    /// Snapshot: verify exact setup_block shape for a DocumentExtractor-like bridge.
    #[test]
    fn test_backend_snapshot() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        let extract_bytes = make_method(
            "extract_bytes",
            vec![
                ("content", TypeRef::Bytes),
                ("mime_type", TypeRef::String),
                ("config", TypeRef::Named("ExtractionConfig".to_string())),
            ],
            TypeRef::Named("HiddenRecord".to_string()),
            false,
        );

        let mut fixture = make_fixture("register_document_extractor_trait_bridge");
        fixture.input = serde_json::json!({
            "extractor": { "type": "test", "name": "test-extractor" }
        });

        let methods = vec![&extract_bytes];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        let expected_setup = concat!(
            "stub_register_document_extractor_trait_bridge = Class.new do\n",
            "  def name = 'test-extractor'\n",
            "  def initialize\n",
            "    nil\n",
            "  end\n",
            "  def shutdown\n",
            "    nil\n",
            "  end\n",
            "  def version = '1.0.0'\n",
            "  def extract_bytes(content, mime_type, config) = '{}'\n",
            "end.new\n",
        );
        assert_eq!(emission.setup_block, expected_setup, "setup_block snapshot mismatch");
        assert_eq!(emission.arg_expr, "stub_register_document_extractor_trait_bridge");
    }
}

#[cfg(test)]
mod gemfile_tests {
    use super::render_gemfile;
    use crate::e2e::config::DependencyMode;

    #[test]
    fn render_gemfile_registry_release_uses_tilde_rocket() {
        let out = render_gemfile("my-gem", "../../packages/ruby", "1.2.3", DependencyMode::Registry);
        assert!(out.contains("gem 'my-gem', '~> 1.2.3'"), "got: {out}");
    }

    #[test]
    fn render_gemfile_registry_prerelease_uses_rubygems_dot_pre_form() {
        let out = render_gemfile("my-gem", "../../packages/ruby", "3.6.0-rc.1", DependencyMode::Registry);
        assert!(
            out.contains("gem 'my-gem', '~> 3.6.0.pre.rc.1'"),
            "pre-release must use .pre. form, got: {out}"
        );
        assert!(
            !out.contains("3.6.0-rc.1"),
            "raw semver dash form must not appear in registry Gemfile, got: {out}"
        );
    }

    #[test]
    fn render_gemfile_registry_already_prefixed_passes_through() {
        // When alef.toml's [crates.e2e.registry.packages.ruby] version field already
        // includes a rubygems operator (`~> 3.6.0.pre.rc.1`), the codegen must use
        // it verbatim — wrapping with another `~> ` produces a double-prefix bug.
        let out = render_gemfile(
            "my-gem",
            "../../packages/ruby",
            "~> 3.6.0.pre.rc.1",
            DependencyMode::Registry,
        );
        assert!(
            out.contains("gem 'my-gem', '~> 3.6.0.pre.rc.1'"),
            "already-prefixed input must pass through verbatim, got: {out}"
        );
        assert!(!out.contains("~> ~>"), "must not double the `~>` prefix, got: {out}");
    }

    #[test]
    fn render_gemfile_local_uses_path() {
        let out = render_gemfile("my-gem", "../../packages/ruby", "3.6.0-rc.1", DependencyMode::Local);
        assert!(out.contains("path: '../../packages/ruby'"), "got: {out}");
        // The target gem line must use path:, not a version constraint.
        assert!(
            out.contains("gem 'my-gem', path:"),
            "local mode must use path: for the target gem, got: {out}"
        );
        assert!(
            !out.contains("gem 'my-gem', '~>"),
            "local mode must not pin a version for the target gem, got: {out}"
        );
    }

    #[test]
    fn app_harness_rb_contains_eaddrinuse_retry_block() {
        use crate::core::config::e2e::{E2eConfig, HarnessConfig};
        use crate::e2e::fixture::{Fixture, FixtureGroup, HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};
        use std::collections::BTreeMap;

        // Build a minimal HTTP fixture so render_app_harness produces server-pattern content.
        let fixture = Fixture {
            id: "test_get".to_owned(),
            description: "test fixture".to_owned(),
            category: Some("smoke".to_owned()),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: "test".to_owned(),
            http: Some(HttpFixture {
                handler: HttpHandler {
                    route: "/test".to_owned(),
                    method: "GET".to_owned(),
                    body_schema: None,
                    parameters: BTreeMap::new(),
                    middleware: None,
                },
                request: HttpRequest {
                    method: "GET".to_owned(),
                    path: "/test".to_owned(),
                    headers: BTreeMap::new(),
                    query_params: BTreeMap::new(),
                    cookies: BTreeMap::new(),
                    body: None,
                    content_type: None,
                },
                expected_response: HttpExpectedResponse {
                    status_code: 200,
                    body: Some(serde_json::json!({"ok": true})),
                    body_partial: None,
                    headers: BTreeMap::new(),
                    validation_errors: None,
                },
            }),
        };

        let groups = vec![FixtureGroup {
            category: "smoke".to_owned(),
            fixtures: vec![fixture],
        }];
        let e2e_config = E2eConfig {
            harness: HarnessConfig {
                imports: vec!["my_gem".to_owned()],
                ..HarnessConfig::default()
            },
            ..E2eConfig::default()
        };

        let out = super::render_app_harness(&e2e_config, &groups);

        // The EADDRINUSE retry block must be present in the generated harness
        assert!(
            out.contains("Errno::EADDRINUSE"),
            "expected `Errno::EADDRINUSE` retry block in generated app_harness.rb:\n{out}"
        );
        // The random port selection must be present
        assert!(
            out.contains("rand(40000..60000)") || out.contains("rand("),
            "expected random port selection in generated app_harness.rb:\n{out}"
        );
        // HARNESS_PORT must be printed so spec_helper can read it
        assert!(
            out.contains("HARNESS_PORT="),
            "expected `HARNESS_PORT=` output in generated app_harness.rb:\n{out}"
        );
    }
}
