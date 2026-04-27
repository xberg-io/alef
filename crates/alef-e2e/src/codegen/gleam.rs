//! Gleam e2e test generator using gleeunit/should.
//!
//! Generates `packages/gleam/test/<crate>_test.gleam` files from JSON fixtures,
//! driven entirely by `E2eConfig` and `CallConfig`.

use crate::config::E2eConfig;
use crate::escape::{escape_gleam, sanitize_filename, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use alef_core::hash::{self, CommentStyle};
use anyhow::Result;
use heck::ToSnakeCase;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// Gleam e2e code generator.
pub struct GleamE2eCodegen;

impl E2eCodegen for GleamE2eCodegen {
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
        let function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        let result_var = &call.result_var;

        // Resolve package config.
        let gleam_pkg = e2e_config.resolve_package("gleam");
        let pkg_path = gleam_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/gleam".to_string());
        let pkg_name = gleam_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| alef_config.crate_config.name.to_snake_case());

        // Generate gleam.toml.
        files.push(GeneratedFile {
            path: output_base.join("gleam.toml"),
            content: render_gleam_toml(&pkg_path, &pkg_name, e2e_config.dep_mode),
            generated_header: false,
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

            let filename = format!("{}_test.gleam", sanitize_filename(&group.category));
            let field_resolver = FieldResolver::new(
                &e2e_config.fields,
                &e2e_config.fields_optional,
                &e2e_config.result_fields,
                &e2e_config.fields_array,
            );
            let content = render_test_file(
                &group.category,
                &active,
                e2e_config,
                &module_path,
                &function_name,
                result_var,
                &e2e_config.call.args,
                &field_resolver,
                &e2e_config.fields_enum,
            );
            files.push(GeneratedFile {
                path: output_base.join("test").join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "gleam"
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_gleam_toml(pkg_path: &str, pkg_name: &str, dep_mode: crate::config::DependencyMode) -> String {
    use alef_core::template_versions::hex;
    let stdlib = hex::GLEAM_STDLIB_VERSION_RANGE;
    let gleeunit = hex::GLEEUNIT_VERSION_RANGE;
    let deps = match dep_mode {
        crate::config::DependencyMode::Registry => {
            format!(
                r#"{pkg_name} = ">= 0.1.0"
gleam_stdlib = "{stdlib}"
gleeunit = "{gleeunit}""#
            )
        }
        crate::config::DependencyMode::Local => {
            format!(
                r#"{pkg_name} = {{ path = "{pkg_path}" }}
gleam_stdlib = "{stdlib}"
gleeunit = "{gleeunit}""#
            )
        }
    };

    format!(
        r#"name = "e2e_gleam"
version = "0.1.0"
target = "erlang"

[dependencies]
{deps}
"#
    )
}

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    _category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    module_path: &str,
    function_name: &str,
    result_var: &str,
    args: &[crate::config::ArgMapping],
    field_resolver: &FieldResolver,
    enum_fields: &HashSet<String>,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let _ = writeln!(out, "import gleeunit");
    let _ = writeln!(out, "import gleeunit/should");
    let _ = writeln!(out, "import {module_path}");
    let _ = writeln!(out);

    // Track which modules we need to import based on assertions used.
    let mut needed_modules: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();

    // First pass: determine which helper modules we need.
    for fixture in fixtures {
        for assertion in &fixture.assertions {
            match assertion.assertion_type.as_str() {
                "contains" | "contains_all" | "not_contains" | "starts_with" | "ends_with" | "min_length" | "max_length" | "contains_any" => {
                    needed_modules.insert("string");
                }
                "not_empty" | "is_empty" | "count_min" | "count_equals" => {
                    needed_modules.insert("list");
                }
                "greater_than" | "less_than" | "greater_than_or_equal" | "less_than_or_equal" => {
                    needed_modules.insert("int");
                }
                _ => {}
            }
        }
    }

    // Emit additional imports.
    for module in &needed_modules {
        let _ = writeln!(out, "import gleam/{module}");
    }

    if !needed_modules.is_empty() {
        let _ = writeln!(out);
    }

    // Each fixture becomes its own test function.
    for fixture in fixtures {
        render_test_case(
            &mut out,
            fixture,
            e2e_config,
            module_path,
            function_name,
            result_var,
            args,
            field_resolver,
            enum_fields,
        );
        let _ = writeln!(out);
    }

    out
}

#[allow(clippy::too_many_arguments)]
fn render_test_case(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    module_path: &str,
    _function_name: &str,
    _result_var: &str,
    _args: &[crate::config::ArgMapping],
    field_resolver: &FieldResolver,
    enum_fields: &HashSet<String>,
) {
    // Resolve per-fixture call config.
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let lang = "gleam";
    let call_overrides = call_config.overrides.get(lang);
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    let result_var = &call_config.result_var;
    let args = &call_config.args;

    let test_name = sanitize_ident(&fixture.id);
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let (setup_lines, args_str) = build_args_and_setup(&fixture.input, args, &fixture.id);

    // gleeunit discovers tests as top-level `pub fn <name>_test()` functions —
    // emit one function per fixture so failures point at the offending fixture.
    let _ = writeln!(out, "// {description}");
    let _ = writeln!(out, "pub fn {test_name}_test() {{");

    for line in &setup_lines {
        let _ = writeln!(out, "  {line}");
    }

    if expects_error {
        let _ = writeln!(
            out,
            "  {module_path}.{function_name}({args_str}) |> should.be_error()"
        );
        let _ = writeln!(out, "}}");
        return;
    }

    let _ = writeln!(
        out,
        "  let {result_var} = {module_path}.{function_name}({args_str})"
    );
    let _ = writeln!(out, "  {result_var} |> should.be_ok()");

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            field_resolver,
            enum_fields,
        );
    }

    let _ = writeln!(out, "}}");
}

/// Build setup lines and the argument list for the function call.
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    fixture_id: &str,
) -> (Vec<String>, String) {
    if args.is_empty() {
        return (Vec::new(), String::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "mock_url" {
            setup_lines.push(format!(
                "let {} = (import \"os\" as os).get_env(\"MOCK_SERVER_URL\") <> \"/fixtures/{fixture_id}\"",
                arg.name,
            ));
            parts.push(arg.name.clone());
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                continue;
            }
            None | Some(serde_json::Value::Null) => {
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "False".to_string(),
                    _ => "Nil".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                parts.push(json_to_gleam(v));
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
    enum_fields: &HashSet<String>,
) {
    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "  // skipped: field '{{f}}' not available on result type");
            return;
        }
    }

    // Determine if this field is an enum type.
    let _field_is_enum = assertion
        .field
        .as_deref()
        .is_some_and(|f| enum_fields.contains(f) || enum_fields.contains(field_resolver.resolve(f)));

    let field_expr = match &assertion.field {
        Some(f) if !f.is_empty() => field_resolver.accessor(f, "gleam", result_var),
        _ => result_var.to_string(),
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let gleam_val = json_to_gleam(expected);
                let _ = writeln!(out, "  {field_expr} |> should.equal({gleam_val})");
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let gleam_val = json_to_gleam(expected);
                let _ = writeln!(
                    out,
                    "  {field_expr} |> string.contains({gleam_val}) |> should.equal(True)"
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let gleam_val = json_to_gleam(val);
                    let _ = writeln!(
                        out,
                        "  {field_expr} |> string.contains({gleam_val}) |> should.equal(True)"
                    );
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let gleam_val = json_to_gleam(expected);
                let _ = writeln!(
                    out,
                    "  {field_expr} |> string.contains({gleam_val}) |> should.equal(False)"
                );
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "  {field_expr} |> list.is_empty |> should.equal(False)");
        }
        "is_empty" => {
            let _ = writeln!(out, "  {field_expr} |> list.is_empty |> should.equal(True)");
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let gleam_val = json_to_gleam(expected);
                let _ = writeln!(
                    out,
                    "  {field_expr} |> string.starts_with({gleam_val}) |> should.equal(True)"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let gleam_val = json_to_gleam(expected);
                let _ = writeln!(
                    out,
                    "  {field_expr} |> string.ends_with({gleam_val}) |> should.equal(True)"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "  {field_expr} |> string.length |> int.is_at_least({n}) |> should.equal(True)"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "  {field_expr} |> string.length |> int.is_at_most({n}) |> should.equal(True)"
                    );
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "  {field_expr} |> list.length |> int.is_at_least({n}) |> should.equal(True)"
                    );
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "  {field_expr} |> list.length |> should.equal({n})"
                    );
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "  {field_expr} |> should.equal(True)");
        }
        "is_false" => {
            let _ = writeln!(out, "  {field_expr} |> should.equal(False)");
        }
        "not_error" => {
            // Already handled by the call succeeding.
        }
        "error" => {
            // Handled at the test case level.
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let gleam_val = json_to_gleam(val);
                let _ = writeln!(out, "  {field_expr} |> int.is_strictly_greater_than({gleam_val}) |> should.equal(True)");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let gleam_val = json_to_gleam(val);
                let _ = writeln!(out, "  {field_expr} |> int.is_strictly_less_than({gleam_val}) |> should.equal(True)");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let gleam_val = json_to_gleam(val);
                let _ = writeln!(out, "  {field_expr} |> int.is_at_least({gleam_val}) |> should.equal(True)");
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let gleam_val = json_to_gleam(val);
                let _ = writeln!(out, "  {field_expr} |> int.is_at_most({gleam_val}) |> should.equal(True)");
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let gleam_val = json_to_gleam(val);
                    let _ = writeln!(
                        out,
                        "  {field_expr} |> string.contains({gleam_val}) |> should.equal(True)"
                    );
                }
            }
        }
        "matches_regex" => {
            let _ = writeln!(out, "  // regex match not yet implemented for Gleam");
        }
        "method_result" => {
            let _ = writeln!(out, "  // method_result assertions not yet implemented for Gleam");
        }
        other => {
            panic!("Gleam e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Convert a `serde_json::Value` to a Gleam literal string.
fn json_to_gleam(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_gleam(s)),
        serde_json::Value::Bool(b) => {
            if *b {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "Nil".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_gleam).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_gleam(&json_str))
        }
    }
}
