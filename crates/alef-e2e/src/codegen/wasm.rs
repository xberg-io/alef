//! WebAssembly e2e test generator using vitest.
//!
//! Similar to the TypeScript generator but imports from a wasm package
//! and uses `language_name` "wasm".

use crate::config::E2eConfig;
use crate::escape::{escape_js, sanitize_filename, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use anyhow::Result;
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// WebAssembly e2e code generator.
pub struct WasmCodegen;

impl E2eCodegen for WasmCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        _alef_config: &AlefConfig,
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);
        let tests_base = output_base.join("tests");

        let mut files = Vec::new();

        // Resolve call config with overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let module_path = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        let function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        let options_type = overrides.and_then(|o| o.options_type.clone());
        let empty_enum_fields = HashMap::new();
        let enum_fields = overrides.map(|o| &o.enum_fields).unwrap_or(&empty_enum_fields);
        let result_var = &call.result_var;
        let is_async = call.r#async;

        // Resolve package config.
        let wasm_pkg = e2e_config.resolve_package("wasm");
        let pkg_path = wasm_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../crates/html-to-markdown-wasm/pkg".to_string());
        let pkg_name = wasm_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| module_path.clone());
        let pkg_version = wasm_pkg
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

        // Generate vitest.config.ts.
        files.push(GeneratedFile {
            path: output_base.join("vitest.config.ts"),
            content: render_vitest_config(),
            generated_header: true,
        });

        // Generate test files per category.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| f.skip.as_ref().is_none_or(|s| !s.should_skip(lang)))
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("{}.test.ts", sanitize_filename(&group.category));
            let field_resolver = FieldResolver::new(
                &e2e_config.fields,
                &e2e_config.fields_optional,
                &e2e_config.result_fields,
                &e2e_config.fields_array,
            );
            let content = render_test_file(
                &group.category,
                &active,
                &pkg_name,
                &function_name,
                result_var,
                is_async,
                &e2e_config.call.args,
                &field_resolver,
                options_type.as_deref(),
                enum_fields,
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
        "wasm"
    }
}

fn render_package_json(
    pkg_name: &str,
    pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::config::DependencyMode,
) -> String {
    let dep_value = match dep_mode {
        crate::config::DependencyMode::Registry => pkg_version.to_string(),
        crate::config::DependencyMode::Local => format!("file:{pkg_path}"),
    };
    format!(
        r#"{{
  "name": "{pkg_name}-e2e-wasm",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {{
    "test": "vitest run"
  }},
  "devDependencies": {{
    "{pkg_name}": "{dep_value}",
    "vite-plugin-top-level-await": "^1.4.0",
    "vite-plugin-wasm": "^3.4.0",
    "vitest": "^3.0.0"
  }}
}}
"#
    )
}

fn render_vitest_config() -> String {
    r#"// This file is auto-generated by alef. DO NOT EDIT.
import { defineConfig } from 'vitest/config';
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';

export default defineConfig({
  plugins: [wasm(), topLevelAwait()],
  test: {
    include: ['tests/**/*.test.ts'],
  },
});
"#
    .to_string()
}

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    pkg_name: &str,
    function_name: &str,
    result_var: &str,
    is_async: bool,
    args: &[crate::config::ArgMapping],
    field_resolver: &FieldResolver,
    options_type: Option<&str>,
    enum_fields: &HashMap<String, String>,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// This file is auto-generated by alef. DO NOT EDIT.");
    let _ = writeln!(out, "import {{ describe, it, expect }} from 'vitest';");

    // Check if any fixture uses a json_object arg that needs the options type import.
    let needs_options_import = options_type.is_some()
        && fixtures.iter().any(|f| {
            args.iter()
                .any(|arg| arg.arg_type == "json_object" && f.input.get(&arg.field).is_some_and(|v| !v.is_null()))
        });

    // Collect all enum types that need to be imported.
    let mut enum_imports: std::collections::BTreeSet<&String> = std::collections::BTreeSet::new();
    if needs_options_import {
        for fixture in fixtures {
            for arg in args {
                if arg.arg_type == "json_object" {
                    if let Some(val) = fixture.input.get(&arg.field) {
                        if let Some(obj) = val.as_object() {
                            for k in obj.keys() {
                                if let Some(enum_type) = enum_fields.get(k) {
                                    enum_imports.insert(enum_type);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Collect handle constructor imports.
    let handle_constructors: Vec<String> = args
        .iter()
        .filter(|arg| arg.arg_type == "handle")
        .map(|arg| format!("create{}", arg.name.to_upper_camel_case()))
        .collect();

    {
        let mut imports = vec![function_name.to_string()];
        imports.extend(handle_constructors);
        if let (true, Some(opts_type)) = (needs_options_import, options_type) {
            imports.push(opts_type.to_string());
            imports.extend(enum_imports.iter().map(|s| s.to_string()));
        }
        let _ = writeln!(out, "import {{ {} }} from '{pkg_name}';", imports.join(", "));
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "describe('{category}', () => {{");

    for (i, fixture) in fixtures.iter().enumerate() {
        render_test_case(
            &mut out,
            fixture,
            function_name,
            result_var,
            is_async,
            args,
            field_resolver,
            options_type,
            enum_fields,
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
    is_async: bool,
    args: &[crate::config::ArgMapping],
    field_resolver: &FieldResolver,
    options_type: Option<&str>,
    enum_fields: &HashMap<String, String>,
) {
    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.replace('\'', "\\'");
    let async_kw = if is_async { "async " } else { "" };
    let await_kw = if is_async { "await " } else { "" };

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let (setup_lines, arg_parts) = build_args_and_setup(&fixture.input, args, options_type, enum_fields, &fixture.id);
    let args_str = arg_parts.join(", ");

    if expects_error {
        let _ = writeln!(out, "  it('{test_name}: {description}', {async_kw}() => {{");
        for line in &setup_lines {
            let _ = writeln!(out, "    {line}");
        }
        if is_async {
            let _ = writeln!(
                out,
                "    await expect({async_kw}() => {await_kw}{function_name}({args_str})).rejects.toThrow();"
            );
        } else {
            let _ = writeln!(out, "    expect(() => {function_name}({args_str})).toThrow();");
        }
        let _ = writeln!(out, "  }});");
        return;
    }

    let _ = writeln!(out, "  it('{test_name}: {description}', {async_kw}() => {{");
    for line in &setup_lines {
        let _ = writeln!(out, "    {line}");
    }
    let _ = writeln!(out, "    const {result_var} = {await_kw}{function_name}({args_str});");

    for assertion in &fixture.assertions {
        render_assertion(out, assertion, result_var, field_resolver);
    }

    let _ = writeln!(out, "  }});");
}

/// Build setup lines and argument parts for a function call.
///
/// Returns `(setup_lines, args_parts)`. Setup lines are emitted before the
/// function call; args parts are joined with `, ` to form the argument list.
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    options_type: Option<&str>,
    enum_fields: &HashMap<String, String>,
    fixture_id: &str,
) -> (Vec<String>, Vec<String>) {
    let mut setup_lines = Vec::new();
    let mut parts = Vec::new();

    if args.is_empty() {
        parts.push(json_to_js(input));
        return (setup_lines, parts);
    }

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
            let constructor_name = format!("create{}", arg.name.to_upper_camel_case());
            let config_value = input.get(&arg.field).unwrap_or(&serde_json::Value::Null);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("const {} = {constructor_name}(null);", arg.name));
            } else {
                let js_val = json_to_js(config_value);
                setup_lines.push(format!("const {} = {constructor_name}({js_val});", arg.name));
            }
            parts.push(arg.name.clone());
            continue;
        }

        let val = input.get(&arg.field);
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => continue,
            None | Some(serde_json::Value::Null) => {
                let default_val = match arg.arg_type.as_str() {
                    "string" => "''".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                if arg.arg_type == "json_object" && !v.is_null() {
                    if let Some(opts_type) = options_type {
                        if let Some(obj) = v.as_object() {
                            setup_lines.push(format!("const options = {opts_type}.default();"));
                            for (k, field_val) in obj {
                                let camel_key = k.to_lower_camel_case();
                                let js_val = if let Some(enum_type) = enum_fields.get(k) {
                                    if let Some(s) = field_val.as_str() {
                                        let pascal_val = s.to_upper_camel_case();
                                        format!("{enum_type}.{pascal_val}")
                                    } else {
                                        json_to_js(field_val)
                                    }
                                } else {
                                    json_to_js(field_val)
                                };
                                setup_lines.push(format!("options.{camel_key} = {js_val};"));
                            }
                            parts.push("options".to_string());
                            continue;
                        }
                    }
                }
                parts.push(json_to_js(v));
            }
        }
    }

    (setup_lines, parts)
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
        Some(f) if !f.is_empty() => field_resolver.accessor(f, "wasm", result_var),
        _ => result_var.to_string(),
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                if expected.is_string() {
                    let _ = writeln!(out, "    expect({field_expr}.trim()).toBe({js_val});");
                } else {
                    let _ = writeln!(out, "    expect({field_expr}).toBe({js_val});");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let _ = writeln!(out, "    expect({field_expr}).toContain({js_val});");
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
            let _ = writeln!(out, "    expect({field_expr}.length).toBeGreaterThan(0);");
        }
        "is_empty" => {
            let _ = writeln!(out, "    expect({field_expr}.trim()).toHaveLength(0);");
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
                let _ = writeln!(out, "    expect({field_expr}.startsWith({js_val})).toBe(true);");
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    expect({field_expr}.length).toBeGreaterThanOrEqual({n});");
                }
            }
        }
        "not_error" => {
            // No-op — if we got here, the call succeeded.
        }
        "error" => {
            // Handled at the test level.
        }
        other => {
            let _ = writeln!(out, "    // TODO: unsupported assertion type: {other}");
        }
    }
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
            let entries: Vec<String> = map.iter().map(|(k, v)| format!("{}: {}", k, json_to_js(v))).collect();
            format!("{{ {} }}", entries.join(", "))
        }
    }
}
