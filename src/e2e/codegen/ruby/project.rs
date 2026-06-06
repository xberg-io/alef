use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;
use crate::core::version::to_rubygems_prerelease;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::ruby_string_literal;
use crate::e2e::fixture::FixtureGroup;
use std::fmt::Write as FmtWrite;

/// Build a Ruby-native middleware value for app harness fixtures.
pub(super) fn build_middleware_value(middleware: &Option<crate::e2e::fixture::HttpMiddleware>) -> serde_json::Value {
    let Some(mw) = middleware else {
        return serde_json::Value::Null;
    };

    let mut map = serde_json::Map::new();

    // --- cors ---
    if let Some(cors) = &mw.cors {
        let mut cors_map = serde_json::Map::new();
        cors_map.insert("allowed_origins".to_string(), serde_json::json!(cors.allow_origins));
        cors_map.insert("allowed_methods".to_string(), serde_json::json!(cors.allow_methods));
        cors_map.insert("allowed_headers".to_string(), serde_json::json!(cors.allow_headers));
        if !cors.expose_headers.is_empty() {
            cors_map.insert("expose_headers".to_string(), serde_json::json!(cors.expose_headers));
        }
        if let Some(max_age) = cors.max_age {
            cors_map.insert("max_age".to_string(), serde_json::json!(max_age));
        }
        if cors.allow_credentials {
            cors_map.insert("allow_credentials".to_string(), serde_json::json!(true));
        }
        map.insert("cors".to_string(), serde_json::Value::Object(cors_map));
    }

    if map.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::Object(map)
    }
}

pub(super) fn render_app_harness(e2e_config: &E2eConfig, groups: &[FixtureGroup]) -> String {
    // Collect all HTTP fixtures from all groups.
    let mut fixtures_map = serde_json::Map::new();

    for group in groups {
        for fixture in &group.fixtures {
            if fixture.http.is_none() {
                continue;
            }
            // Convert the fixture to JSON for the harness to load.
            let http_data = &fixture.http.as_ref().unwrap();
            let middleware_value = build_middleware_value(&http_data.handler.middleware);
            let fixture_json = serde_json::json!({
                "http": {
                    "handler": {
                        "route": &http_data.handler.route,
                        "method": &http_data.handler.method,
                        "body_schema": http_data.handler.body_schema.clone(),
                        "middleware": middleware_value,
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
        format!("{}::", super::values::ruby_module_name(&imports_ref[0]))
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

pub(super) fn render_gemfile(
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
pub(super) fn render_spec_helper(
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
    let _module_name = super::values::ruby_module_name(module_path);

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
            harness_host, harness_host, harness_host,
        );
        out.push_str(&harness_setup);
    } else if has_mock_server_fixtures {
        let mock_server_block =
            crate::e2e::template_env::render("ruby/spec_helper_mock_server.rb.jinja", minijinja::context! {});
        out.push_str(&mock_server_block);
    }

    out
}

pub(super) fn render_rubocop_yaml() -> String {
    crate::e2e::template_env::render("ruby/rubocop.yml.jinja", minijinja::context! {})
}
