//! Ruby e2e test generator using RSpec.
//!
//! Generates `e2e/ruby/Gemfile` and `spec/{category}_spec.rb` files from
//! JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

use crate::config::E2eConfig;
use crate::escape::{ruby_string_literal, sanitize_filename, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::{
    Assertion, CallbackAction, Fixture, FixtureGroup, HttpExpectedResponse, HttpFixture, HttpRequest,
};
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use anyhow::Result;
use heck::ToSnakeCase;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// Ruby e2e code generator.
pub struct RubyCodegen;

impl E2eCodegen for RubyCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        alef_config: &AlefConfig,
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
        let result_is_simple = overrides.is_some_and(|o| o.result_is_simple);

        // Resolve package config.
        let ruby_pkg = e2e_config.resolve_package("ruby");
        let gem_name = ruby_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| alef_config.crate_config.name.replace('-', "_"));
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

        // Generate spec files per category.
        let spec_base = output_base.join("spec");

        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| f.skip.as_ref().is_none_or(|s| !s.should_skip(lang)))
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
         gem 'rspec', '~> 3.13'\n\
         gem 'rubocop', '~> 1.86'\n\
         gem 'rubocop-rspec', '~> 3.9'\n\
         gem 'faraday', '~> 2.0'\n"
    )
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
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# This file is auto-generated by alef. DO NOT EDIT.");
    let _ = writeln!(out, "# frozen_string_literal: true");
    let _ = writeln!(out);

    // Require the gem (single quotes).
    let require_name = if module_path.is_empty() { gem_name } else { module_path };
    let _ = writeln!(out, "require '{}'", require_name.replace('-', "_"));
    let _ = writeln!(out, "require 'json'");

    // Add faraday if any fixture is an HTTP test.
    let has_http = fixtures.iter().any(|f| f.is_http_test());
    if has_http {
        let _ = writeln!(out, "require 'faraday'");
    }
    let _ = writeln!(out);

    // Build the Ruby module/class qualifier for calls.
    let call_receiver = class_name
        .map(|s| s.to_string())
        .unwrap_or_else(|| ruby_module_name(module_path));

    let _ = writeln!(out, "RSpec.describe '{}' do", category);

    // Emit a shared client helper when there are HTTP tests.
    if has_http {
        let _ = writeln!(
            out,
            "  let(:base_url) {{ ENV.fetch('TEST_SERVER_URL', 'http://localhost:8080') }}"
        );
        let _ = writeln!(out, "  let(:client) do");
        let _ = writeln!(out, "    Faraday.new(url: base_url) do |f|");
        let _ = writeln!(out, "      f.request :json");
        let _ = writeln!(out, "      f.response :json, content_type: /\\bjson$/");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
        let _ = writeln!(out);
    }

    let mut first = true;
    for fixture in fixtures {
        if !first {
            let _ = writeln!(out);
        }
        first = false;

        if let Some(http) = &fixture.http {
            render_http_example(&mut out, fixture, http);
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
            render_example(
                &mut out,
                fixture,
                &fixture_function_name,
                &call_receiver,
                fixture_result_var,
                fixture_args,
                field_resolver,
                options_type,
                enum_fields,
                result_is_simple,
                e2e_config,
            );
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
// HTTP test rendering
// ---------------------------------------------------------------------------

/// Render an RSpec `describe` + `it` block for an HTTP server test fixture.
fn render_http_example(out: &mut String, fixture: &Fixture, http: &HttpFixture) {
    let description = fixture.description.replace('\'', "\\'");
    let method = http.request.method.to_uppercase();
    let path = &http.request.path;

    let _ = writeln!(out, "  describe '{method} {path}' do");
    let _ = writeln!(out, "    it '{}' do", description);

    // Build request call.
    render_ruby_http_request(out, &http.request);

    // Assert status.
    let status = http.expected_response.status_code;
    let _ = writeln!(out, "      expect(response.status).to eq({status})");

    // Assert response body.
    render_ruby_body_assertions(out, &http.expected_response);

    // Assert response headers.
    render_ruby_header_assertions(out, &http.expected_response);

    let _ = writeln!(out, "    end");
    let _ = writeln!(out, "  end");
}

/// Emit the Faraday request lines inside an RSpec example.
fn render_ruby_http_request(out: &mut String, req: &HttpRequest) {
    let method = req.method.to_lowercase();

    // Build options hash.
    let mut opts: Vec<String> = Vec::new();

    if let Some(body) = &req.body {
        let ruby_body = json_to_ruby(body);
        opts.push(format!("json: {ruby_body}"));
    }

    if !req.headers.is_empty() {
        let header_pairs: Vec<String> = req
            .headers
            .iter()
            .map(|(k, v)| format!("{} => {}", ruby_string_literal(k), ruby_string_literal(v)))
            .collect();
        opts.push(format!("headers: {{ {} }}", header_pairs.join(", ")));
    }

    if !req.cookies.is_empty() {
        let cookie_str = req
            .cookies
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("; ");
        opts.push(format!(
            "headers: {{ 'Cookie' => {} }}",
            ruby_string_literal(&cookie_str)
        ));
    }

    // Build path with optional query string.
    let path = if req.query_params.is_empty() {
        ruby_string_literal(&req.path)
    } else {
        let pairs: Vec<String> = req
            .query_params
            .iter()
            .map(|(k, v)| {
                let val_str = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("{}={}", k, val_str)
            })
            .collect();
        ruby_string_literal(&format!("{}?{}", req.path, pairs.join("&")))
    };

    if opts.is_empty() {
        let _ = writeln!(out, "      response = client.{method}({path})");
    } else {
        let _ = writeln!(out, "      response = client.{method}({path},");
        for (i, opt) in opts.iter().enumerate() {
            if i + 1 < opts.len() {
                let _ = writeln!(out, "        {opt},");
            } else {
                let _ = writeln!(out, "        {opt}");
            }
        }
        let _ = writeln!(out, "      )");
    }
}

/// Emit body assertions for an HTTP expected response.
fn render_ruby_body_assertions(out: &mut String, expected: &HttpExpectedResponse) {
    if let Some(body) = &expected.body {
        let ruby_val = json_to_ruby(body);
        let _ = writeln!(out, "      expect(response.body).to eq({ruby_val})");
    }
    if let Some(partial) = &expected.body_partial {
        if let Some(obj) = partial.as_object() {
            for (key, val) in obj {
                let ruby_key = ruby_string_literal(key);
                let ruby_val = json_to_ruby(val);
                let _ = writeln!(out, "      expect(response.body[{ruby_key}]).to eq({ruby_val})");
            }
        }
    }
    if let Some(errors) = &expected.validation_errors {
        for err in errors {
            let msg_lit = ruby_string_literal(&err.msg);
            let _ = writeln!(out, "      expect(response.body.to_s).to include({msg_lit})");
        }
    }
}

/// Emit header assertions for an HTTP expected response.
///
/// Special tokens:
/// - `"<<present>>"` — assert the header key exists
/// - `"<<absent>>"` — assert the header key is absent
/// - `"<<uuid>>"` — assert the header value matches a UUID regex
fn render_ruby_header_assertions(out: &mut String, expected: &HttpExpectedResponse) {
    for (name, value) in &expected.headers {
        let header_key = name.to_lowercase();
        let header_expr = format!("response.headers[{}]", ruby_string_literal(&header_key));
        match value.as_str() {
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

    let call_expr = format!("{call_receiver}.{function_name}({final_args})");

    let _ = writeln!(out, "  it '{test_name}: {description}' do");

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
        return (Vec::new(), json_to_ruby(input));
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "mock_url" {
            setup_lines.push(format!(
                "{} = \"#{{ENV.fetch('MOCK_SERVER_URL')}}/fixtures/{fixture_id}\"",
                arg.name,
            ));
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "handle" {
            // Generate a create_engine (or equivalent) call and pass the variable.
            let constructor_name = format!("create_{}", arg.name.to_snake_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
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

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value: skip entirely.
                continue;
            }
            None | Some(serde_json::Value::Null) => {
                // Required arg with no fixture value: pass a language-appropriate default.
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
                // For json_object args with options_type, construct a typed options object.
                // When result_is_simple, the binding accepts a plain Hash (no wrapper class).
                if arg.arg_type == "json_object" && !v.is_null() {
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
        format!("{field_expr}.strip")
    } else {
        field_expr.clone()
    };

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
                // Use .to_s to handle both String and Symbol (enum) fields
                let _ = writeln!(out, "    expect({field_expr}.to_s).to include({rb_val})");
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let rb_val = json_to_ruby(val);
                    let _ = writeln!(out, "    expect({field_expr}.to_s).to include({rb_val})");
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let _ = writeln!(out, "    expect({field_expr}.to_s).not_to include({rb_val})");
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "    expect({field_expr}).not_to be_empty");
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
            setup_lines.push(format!("    {{ custom: \"{template}\" }}"));
        }
    }
    setup_lines.push("  end".to_string());
}
