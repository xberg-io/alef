//! TypeScript e2e test generator using vitest.

use crate::config::E2eConfig;
use crate::escape::{escape_js, sanitize_filename, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use anyhow::Result;
use heck::ToUpperCamelCase;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// TypeScript e2e code generator.
pub struct TypeScriptCodegen;

impl E2eCodegen for TypeScriptCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        _alef_config: &AlefConfig,
    ) -> Result<Vec<GeneratedFile>> {
        let output_base = PathBuf::from(e2e_config.effective_output()).join(self.language_name());
        let tests_base = output_base.join("tests");

        let mut files = Vec::new();

        // Resolve call config with overrides — use "node" key (Language::Node).
        let call = &e2e_config.call;
        let overrides = call.overrides.get("node");
        let module_path = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        let function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        let result_var = &call.result_var;
        let is_async = call.r#async;
        let client_factory = overrides.and_then(|o| o.client_factory.as_deref());

        // Resolve package config.
        let node_pkg = e2e_config.resolve_package("node");
        let pkg_path = node_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/typescript".to_string());
        let pkg_name = node_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| module_path.clone());
        let pkg_version = node_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .unwrap_or_else(|| "0.1.0".to_string());

        // Generate package.json.
        files.push(GeneratedFile {
            path: output_base.join("package.json"),
            content: render_package_json(&pkg_name, &pkg_path, &pkg_version, e2e_config.dep_mode),
            generated_header: false,
        });

        // Generate tsconfig.json.
        files.push(GeneratedFile {
            path: output_base.join("tsconfig.json"),
            content: render_tsconfig(),
            generated_header: false,
        });

        // Generate vitest.config.ts — include globalSetup when client_factory is used.
        files.push(GeneratedFile {
            path: output_base.join("vitest.config.ts"),
            content: render_vitest_config(client_factory.is_some()),
            generated_header: true,
        });

        // Generate globalSetup.ts when client_factory is used (needs mock server).
        if client_factory.is_some() {
            files.push(GeneratedFile {
                path: output_base.join("globalSetup.ts"),
                content: render_global_setup(),
                generated_header: true,
            });
        }

        // Resolve options_type from override.
        let options_type = overrides.and_then(|o| o.options_type.clone());
        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
        );

        // Generate test files per category.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| f.skip.as_ref().is_none_or(|s| !s.should_skip("node")))
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("{}.test.ts", sanitize_filename(&group.category));
            let content = render_test_file(
                &group.category,
                &active,
                &module_path,
                &pkg_name,
                &function_name,
                result_var,
                is_async,
                &e2e_config.call.args,
                options_type.as_deref(),
                &field_resolver,
                client_factory,
            );
            files.push(GeneratedFile {
                path: tests_base.join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "node"
    }
}

fn render_package_json(
    pkg_name: &str,
    _pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::config::DependencyMode,
) -> String {
    let dep_value = match dep_mode {
        crate::config::DependencyMode::Registry => pkg_version.to_string(),
        crate::config::DependencyMode::Local => "workspace:*".to_string(),
    };
    format!(
        r#"{{
  "name": "{pkg_name}-e2e-typescript",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {{
    "test": "vitest run"
  }},
  "devDependencies": {{
    "{pkg_name}": "{dep_value}",
    "vitest": "^3.0.0"
  }}
}}
"#
    )
}

fn render_tsconfig() -> String {
    r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "strictNullChecks": false,
    "esModuleInterop": true,
    "skipLibCheck": true
  },
  "include": ["tests/**/*.ts", "vitest.config.ts"]
}
"#
    .to_string()
}

fn render_vitest_config(with_global_setup: bool) -> String {
    if with_global_setup {
        r#"// This file is auto-generated by alef. DO NOT EDIT.
import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['tests/**/*.test.ts'],
    globalSetup: './globalSetup.ts',
  },
});
"#
        .to_string()
    } else {
        r#"// This file is auto-generated by alef. DO NOT EDIT.
import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['tests/**/*.test.ts'],
  },
});
"#
        .to_string()
    }
}

fn render_global_setup() -> String {
    r#"// This file is auto-generated by alef. DO NOT EDIT.
import { spawn } from 'child_process';
import { resolve } from 'path';

let serverProcess;

export async function setup() {
  // Mock server binary must be pre-built (e.g. by CI or `cargo build --manifest-path e2e/rust/Cargo.toml --bin mock-server --release`)
  serverProcess = spawn(
    resolve(__dirname, '../rust/target/release/mock-server'),
    [resolve(__dirname, '../../fixtures')],
    { stdio: ['pipe', 'pipe', 'inherit'] }
  );

  const url = await new Promise((resolve, reject) => {
    serverProcess.stdout.on('data', (data) => {
      const match = data.toString().match(/MOCK_SERVER_URL=(.*)/);
      if (match) resolve(match[1].trim());
    });
    setTimeout(() => reject(new Error('Mock server startup timeout')), 30000);
  });

  process.env.MOCK_SERVER_URL = url;
}

export async function teardown() {
  if (serverProcess) {
    serverProcess.stdin.end();
    serverProcess.kill();
  }
}
"#
    .to_string()
}

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    module_path: &str,
    pkg_name: &str,
    function_name: &str,
    result_var: &str,
    is_async: bool,
    args: &[crate::config::ArgMapping],
    options_type: Option<&str>,
    field_resolver: &FieldResolver,
    client_factory: Option<&str>,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// This file is auto-generated by alef. DO NOT EDIT.");
    let _ = writeln!(out, "import {{ describe, it, expect }} from 'vitest';");

    // Check if any fixture uses a json_object arg that needs the options type import.
    let needs_options_import = options_type.is_some()
        && fixtures.iter().any(|f| {
            args.iter().any(|arg| {
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                let val = if field == "input" {
                    Some(&f.input)
                } else {
                    f.input.get(field)
                };
                arg.arg_type == "json_object" && val.is_some_and(|v| !v.is_null())
            })
        });

    // Collect handle constructor function names that need to be imported.
    let handle_constructors: Vec<String> = args
        .iter()
        .filter(|arg| arg.arg_type == "handle")
        .map(|arg| format!("create{}", arg.name.to_upper_camel_case()))
        .collect();

    // When using client_factory, import the factory instead of the function.
    let mut imports: Vec<String> = if let Some(factory) = client_factory {
        vec![factory.to_string()]
    } else {
        vec![function_name.to_string()]
    };
    for ctor in &handle_constructors {
        if !imports.contains(ctor) {
            imports.push(ctor.clone());
        }
    }

    // Use pkg_name (the npm package name, e.g. "@kreuzberg/liter-llm") for
    // the import specifier so that registry builds resolve the published package name.
    let _ = module_path; // retained in signature for potential future use
    if let (true, Some(opts_type)) = (needs_options_import, options_type) {
        imports.push(format!("type {opts_type}"));
        let imports_str = imports.join(", ");
        let _ = writeln!(out, "import {{ {imports_str} }} from '{pkg_name}';");
    } else {
        let imports_str = imports.join(", ");
        let _ = writeln!(out, "import {{ {imports_str} }} from '{pkg_name}';");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "describe('{category}', () => {{");

    for (i, fixture) in fixtures.iter().enumerate() {
        render_test_case(
            &mut out,
            fixture,
            function_name,
            result_var,
            client_factory,
            is_async,
            args,
            options_type,
            field_resolver,
        );
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    let _ = writeln!(out, "}});");
    out
}

#[allow(clippy::too_many_arguments)]
fn render_test_case(
    out: &mut String,
    fixture: &Fixture,
    function_name: &str,
    result_var: &str,
    client_factory: Option<&str>,
    is_async: bool,
    args: &[crate::config::ArgMapping],
    options_type: Option<&str>,
    field_resolver: &FieldResolver,
) {
    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.replace('\'', "\\'");
    let async_kw = if is_async { "async " } else { "" };
    let await_kw = if is_async { "await " } else { "" };

    // Build the call expression — either `client.method(args)` or `method(args)`
    let (setup_lines, args_str) = build_args_and_setup(&fixture.input, args, options_type, &fixture.id);
    let call_expr = if client_factory.is_some() {
        format!("client.{function_name}({args_str})")
    } else {
        format!("{function_name}({args_str})")
    };

    // Build the base_url expression for mock server
    let base_url_expr = format!("`${{process.env.MOCK_SERVER_URL}}/fixtures/{}`", fixture.id);

    // Check if this is an error-expecting test.
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    if expects_error {
        let _ = writeln!(out, "  it('{test_name}: {description}', async () => {{");
        if let Some(factory) = client_factory {
            let _ = writeln!(out, "    const client = {factory}('test-key', {base_url_expr});");
        }
        // Wrap ALL setup lines and the function call inside the expect block so that
        // synchronous throws from handle constructors (e.g. createEngine) are also caught.
        let _ = writeln!(out, "    await expect(async () => {{");
        for line in &setup_lines {
            let _ = writeln!(out, "      {line}");
        }
        let _ = writeln!(out, "      await {call_expr};");
        let _ = writeln!(out, "    }}).rejects.toThrow();");
        let _ = writeln!(out, "  }});");
        return;
    }

    let _ = writeln!(out, "  it('{test_name}: {description}', {async_kw}() => {{");

    if let Some(factory) = client_factory {
        let _ = writeln!(out, "    const client = {factory}('test-key', {base_url_expr});");
    }

    for line in &setup_lines {
        let _ = writeln!(out, "    {line}");
    }

    // Check if any assertion actually uses the result variable.
    let has_usable_assertion = fixture.assertions.iter().any(|a| {
        if a.assertion_type == "not_error" || a.assertion_type == "error" {
            return false;
        }
        match &a.field {
            Some(f) if !f.is_empty() => field_resolver.is_valid_for_result(f),
            _ => true,
        }
    });

    if has_usable_assertion {
        let _ = writeln!(out, "    const {result_var} = {await_kw}{call_expr};");
    } else {
        let _ = writeln!(out, "    {await_kw}{call_expr};");
    }

    // Emit assertions.
    for assertion in &fixture.assertions {
        render_assertion(out, assertion, result_var, field_resolver);
    }

    let _ = writeln!(out, "  }});");
}

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// Returns `(setup_lines, args_string)`.
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    options_type: Option<&str>,
    fixture_id: &str,
) -> (Vec<String>, String) {
    if args.is_empty() {
        // If no args mapping, pass the whole input as a single argument.
        return (Vec::new(), json_to_js(input));
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "mock_url" {
            setup_lines.push(format!(
                "const {} = `${{process.env.MOCK_SERVER_URL}}/fixtures/{fixture_id}`;",
                arg.name,
            ));
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "handle" {
            // Generate a createEngine (or equivalent) call and pass the variable.
            let constructor_name = format!("create{}", arg.name.to_upper_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("const {} = {constructor_name}(null);", arg.name));
            } else {
                // NAPI-RS bindings use camelCase for JS field names, so convert snake_case
                // config keys from the fixture JSON to camelCase before passing to the constructor.
                let literal = json_to_js_camel(config_value);
                setup_lines.push(format!("const {name}Config = {literal};", name = arg.name,));
                setup_lines.push(format!(
                    "const {} = {constructor_name}({name}Config);",
                    arg.name,
                    name = arg.name,
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        // When field == "input", the entire input object IS the value (not a nested key)
        let val = if field == "input" {
            Some(input)
        } else {
            input.get(field)
        };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value: skip entirely.
                continue;
            }
            None | Some(serde_json::Value::Null) => {
                // Required arg with no fixture value: pass a language-appropriate default.
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                // For json_object args with options_type, cast the object literal.
                if arg.arg_type == "json_object" {
                    if let Some(opts_type) = options_type {
                        parts.push(format!("{} as {opts_type}", json_to_js(v)));
                        continue;
                    }
                }
                parts.push(json_to_js(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
}

fn render_assertion(out: &mut String, assertion: &Assertion, result_var: &str, field_resolver: &FieldResolver) {
    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "    // skipped: field '{f}' not available on result type");
            return;
        }
    }

    let field_expr = match &assertion.field {
        Some(f) if !f.is_empty() => field_resolver.accessor(f, "typescript", result_var),
        _ => result_var.to_string(),
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                // For string equality, trim trailing whitespace to handle trailing newlines
                // from the converter. Use null-coalescing for optional fields.
                if expected.is_string() {
                    let resolved = assertion.field.as_deref().unwrap_or("");
                    if !resolved.is_empty() && field_resolver.is_optional(field_resolver.resolve(resolved)) {
                        let _ = writeln!(out, "    expect(({field_expr} ?? \"\").trim()).toBe({js_val});");
                    } else {
                        let _ = writeln!(out, "    expect({field_expr}.trim()).toBe({js_val});");
                    }
                } else {
                    let _ = writeln!(out, "    expect({field_expr}).toBe({js_val});");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                // Use null-coalescing for optional string fields to handle null/undefined values.
                let resolved = assertion.field.as_deref().unwrap_or("");
                if !resolved.is_empty()
                    && expected.is_string()
                    && field_resolver.is_optional(field_resolver.resolve(resolved))
                {
                    let _ = writeln!(out, "    expect({field_expr} ?? \"\").toContain({js_val});");
                } else {
                    let _ = writeln!(out, "    expect({field_expr}).toContain({js_val});");
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let js_val = json_to_js(val);
                    let _ = writeln!(out, "    expect({field_expr}).toContain({js_val});");
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let _ = writeln!(out, "    expect({field_expr}).not.toContain({js_val});");
            }
        }
        "not_empty" => {
            // Use null-coalescing for optional fields to handle null/undefined values.
            let resolved = assertion.field.as_deref().unwrap_or("");
            if !resolved.is_empty() && field_resolver.is_optional(field_resolver.resolve(resolved)) {
                let _ = writeln!(out, "    expect(({field_expr} ?? \"\").length).toBeGreaterThan(0);");
            } else {
                let _ = writeln!(out, "    expect({field_expr}.length).toBeGreaterThan(0);");
            }
        }
        "is_empty" => {
            // Use null-coalescing for optional string fields to handle null/undefined values.
            let resolved = assertion.field.as_deref().unwrap_or("");
            if !resolved.is_empty() && field_resolver.is_optional(field_resolver.resolve(resolved)) {
                let _ = writeln!(out, "    expect({field_expr} ?? \"\").toHaveLength(0);");
            } else {
                let _ = writeln!(out, "    expect({field_expr}).toHaveLength(0);");
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(json_to_js).collect();
                let arr_str = items.join(", ");
                let _ = writeln!(
                    out,
                    "    expect([{arr_str}].some((v) => {field_expr}.includes(v))).toBe(true);"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let _ = writeln!(out, "    expect({field_expr}).toBeGreaterThan({js_val});");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let _ = writeln!(out, "    expect({field_expr}).toBeLessThan({js_val});");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let _ = writeln!(out, "    expect({field_expr}).toBeGreaterThanOrEqual({js_val});");
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let _ = writeln!(out, "    expect({field_expr}).toBeLessThanOrEqual({js_val});");
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                // Use null-coalescing for optional fields to handle null/undefined values.
                let resolved = assertion.field.as_deref().unwrap_or("");
                if !resolved.is_empty() && field_resolver.is_optional(field_resolver.resolve(resolved)) {
                    let _ = writeln!(
                        out,
                        "    expect(({field_expr} ?? \"\").startsWith({js_val})).toBe(true);"
                    );
                } else {
                    let _ = writeln!(out, "    expect({field_expr}.startsWith({js_val})).toBe(true);");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    expect({field_expr}.length).toBeGreaterThanOrEqual({n});");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    expect({field_expr}.length).toBe({n});");
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "    expect({field_expr}).toBe(true);");
        }
        "not_error" => {
            // No-op — if we got here, the call succeeded (it would have thrown).
        }
        "error" => {
            // Handled at the test level (early return above).
        }
        other => {
            let _ = writeln!(out, "    // TODO: unsupported assertion type: {other}");
        }
    }
}

/// Convert a `serde_json::Value` to a JavaScript literal string with camelCase object keys.
///
/// NAPI-RS bindings use camelCase for JavaScript field names. This variant converts
/// snake_case object keys (as written in fixture JSON) to camelCase so that the
/// generated config objects match the NAPI binding's expected field names.
fn json_to_js_camel(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let camel_key = snake_to_camel(k);
                    // Quote keys that aren't valid JS identifiers.
                    let key = if camel_key
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
                        && !camel_key.starts_with(|c: char| c.is_ascii_digit())
                    {
                        camel_key.clone()
                    } else {
                        format!("\"{}\"", escape_js(&camel_key))
                    };
                    format!("{key}: {}", json_to_js_camel(v))
                })
                .collect();
            format!("{{ {} }}", entries.join(", "))
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_js_camel).collect();
            format!("[{}]", items.join(", "))
        }
        // Scalars and null delegate to the standard converter.
        other => json_to_js(other),
    }
}

/// Convert a snake_case string to camelCase.
fn snake_to_camel(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = false;
    for ch in s.chars() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

/// Convert a `serde_json::Value` to a JavaScript literal string.
fn json_to_js(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_js(s)),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_js).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    // Quote keys that aren't valid JS identifiers (contain hyphens, spaces, etc.)
                    let key = if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
                        && !k.starts_with(|c: char| c.is_ascii_digit())
                    {
                        k.clone()
                    } else {
                        format!("\"{}\"", escape_js(k))
                    };
                    format!("{key}: {}", json_to_js(v))
                })
                .collect();
            format!("{{ {} }}", entries.join(", "))
        }
    }
}
