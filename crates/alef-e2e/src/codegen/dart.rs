//! Dart e2e test generator using package:test.
//!
//! Generates `packages/dart/test/<fixture_id>_test.dart` files from JSON
//! fixtures (one file per fixture group, mirroring the Gleam per-fixture-file
//! layout) and a `pubspec.yaml` at the e2e package root.

use crate::config::E2eConfig;
use crate::escape::sanitize_filename;
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::template_versions::pub_dev;
use anyhow::Result;
use heck::{ToLowerCamelCase, ToSnakeCase};
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// Dart e2e code generator.
pub struct DartE2eCodegen;

impl E2eCodegen for DartE2eCodegen {
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
        let function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        let result_var = &call.result_var;
        let result_is_simple = overrides.is_some_and(|o| o.result_is_simple);

        // Resolve package config.
        let dart_pkg = e2e_config.resolve_package("dart");
        // Match the canonical pubspec name used by `dart_pubspec_name()` so the
        // import `package:<pkg>/<module>.dart` resolves consistently across
        // scaffold, publish, and e2e.
        let pkg_name = dart_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| alef_config.dart_pubspec_name());
        let pkg_path = dart_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/dart".to_string());
        let pkg_version = dart_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .unwrap_or_else(|| "0.1.0".to_string());

        // Generate pubspec.yaml.
        files.push(GeneratedFile {
            path: output_base.join("pubspec.yaml"),
            content: render_pubspec(&pkg_name, &pkg_path, &pkg_version, e2e_config.dep_mode),
            generated_header: false,
        });

        let test_base = output_base.join("test");

        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
        );

        // One test file per fixture group.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| f.skip.as_ref().is_none_or(|s| !s.should_skip(lang)))
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("{}_test.dart", sanitize_filename(&group.category));
            let content = render_test_file(
                &group.category,
                &active,
                e2e_config,
                &pkg_name,
                &function_name,
                result_var,
                &e2e_config.call.args,
                &field_resolver,
                result_is_simple,
                &e2e_config.fields_enum,
            );
            files.push(GeneratedFile {
                path: test_base.join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "dart"
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_pubspec(
    pkg_name: &str,
    pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::config::DependencyMode,
) -> String {
    let test_ver = pub_dev::TEST_PACKAGE;

    let dep_block = match dep_mode {
        crate::config::DependencyMode::Registry => {
            format!("  {pkg_name}: ^{pkg_version}")
        }
        crate::config::DependencyMode::Local => {
            format!(
                "  {pkg_name}:\n    path: {pkg_path}"
            )
        }
    };

    format!(
        r#"name: e2e_dart
version: 0.1.0
publish_to: none

environment:
  sdk: ">=3.0.0 <4.0.0"

dependencies:
{dep_block}

dev_dependencies:
  test: {test_ver}
"#
    )
}

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    pkg_name: &str,
    function_name: &str,
    result_var: &str,
    args: &[crate::config::ArgMapping],
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &HashSet<String>,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let module_name = pkg_name.to_snake_case();
    // mock_url args reference Platform.environment which lives in dart:io.
    let needs_dart_io = args.iter().any(|a| a.arg_type == "mock_url");
    let _ = writeln!(out, "import 'package:test/test.dart';");
    if needs_dart_io {
        let _ = writeln!(out, "import 'dart:io';");
    }
    let _ = writeln!(out, "import 'package:{module_name}/{module_name}.dart';");
    let _ = writeln!(out);

    let _ = writeln!(out, "// E2e tests for category: {category}");
    let _ = writeln!(out, "void main() {{");

    for fixture in fixtures {
        render_test_case(
            &mut out,
            fixture,
            e2e_config,
            function_name,
            result_var,
            args,
            field_resolver,
            result_is_simple,
            enum_fields,
        );
    }

    let _ = writeln!(out, "}}");
    out
}

#[allow(clippy::too_many_arguments)]
fn render_test_case(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    _function_name: &str,
    _result_var: &str,
    _args: &[crate::config::ArgMapping],
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    enum_fields: &HashSet<String>,
) {
    // Resolve per-fixture call config.
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let lang = "dart";
    let call_overrides = call_config.overrides.get(lang);
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.to_lower_camel_case());
    let result_var = &call_config.result_var;
    let args = &call_config.args;

    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let is_async = call_config.r#async;

    let (setup_lines, args_str) = build_args_and_setup(&fixture.input, args, &fixture.id);

    // Async tests must always use `async` callbacks — `expect(throwsA(...))` on a
    // synchronous lambda wrapping an async call drops the rejection. Use
    // `expectLater` + `throwsA` for async-error fixtures.
    if is_async {
        let _ = writeln!(out, "  test('{description}', () async {{");
    } else {
        let _ = writeln!(out, "  test('{description}', () {{");
    }

    for line in &setup_lines {
        let _ = writeln!(out, "    {line}");
    }

    if expects_error {
        if is_async {
            let _ = writeln!(
                out,
                "    await expectLater({function_name}({args_str}), throwsA(isA<Exception>()));"
            );
        } else {
            let _ = writeln!(
                out,
                "    expect(() => {function_name}({args_str}), throwsA(isA<Exception>()));"
            );
        }
        let _ = writeln!(out, "  }});");
        let _ = writeln!(out);
        return;
    }

    if is_async {
        let _ = writeln!(
            out,
            "    final {result_var} = await {function_name}({args_str});"
        );
    } else {
        let _ = writeln!(
            out,
            "    final {result_var} = {function_name}({args_str});"
        );
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            field_resolver,
            result_is_simple,
            enum_fields,
        );
    }

    let _ = writeln!(out, "  }});");
    let _ = writeln!(out);
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
                "final {} = Platform.environment['MOCK_SERVER_URL']! + '/fixtures/{fixture_id}';",
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
                    "string" => "''".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                parts.push(json_to_dart(v));
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
    enum_fields: &HashSet<String>,
) {
    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "    // skipped: field '{{f}}' not available on result type");
            return;
        }
    }

    // Determine if this field is an enum type.
    let field_is_enum = assertion
        .field
        .as_deref()
        .is_some_and(|f| enum_fields.contains(f) || enum_fields.contains(field_resolver.resolve(f)));

    let field_expr = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "dart", result_var),
            _ => result_var.to_string(),
        }
    };

    // For enum fields, use .name to get the string value.
    let string_expr = if field_is_enum {
        format!("{field_expr}.name")
    } else {
        field_expr.clone()
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let dart_val = json_to_dart(expected);
                if expected.is_string() {
                    let _ = writeln!(out, "    expect({string_expr}.trim(), equals({dart_val}));");
                } else {
                    let _ = writeln!(out, "    expect({field_expr}, equals({dart_val}));");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let dart_val = json_to_dart(expected);
                let _ = writeln!(out, "    expect({string_expr}, contains({dart_val}));");
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let dart_val = json_to_dart(val);
                    let _ = writeln!(out, "    expect({string_expr}, contains({dart_val}));");
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let dart_val = json_to_dart(expected);
                let _ = writeln!(
                    out,
                    "    expect({string_expr}, isNot(contains({dart_val})));"
                );
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "    expect({field_expr}, isNotEmpty);");
        }
        "is_empty" => {
            let _ = writeln!(out, "    expect({field_expr}, isEmpty);");
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let checks: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let dart_val = json_to_dart(v);
                        format!("{string_expr}.contains({dart_val})")
                    })
                    .collect();
                let joined = checks.join(" || ");
                let _ = writeln!(
                    out,
                    "    expect({joined}, isTrue, reason: 'expected to contain at least one of the specified values');"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let dart_val = json_to_dart(val);
                let _ = writeln!(out, "    expect({field_expr}, greaterThan({dart_val}));");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let dart_val = json_to_dart(val);
                let _ = writeln!(out, "    expect({field_expr}, lessThan({dart_val}));");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let dart_val = json_to_dart(val);
                let _ = writeln!(
                    out,
                    "    expect({field_expr}, greaterThanOrEqualTo({dart_val}));"
                );
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let dart_val = json_to_dart(val);
                let _ = writeln!(
                    out,
                    "    expect({field_expr}, lessThanOrEqualTo({dart_val}));"
                );
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let dart_val = json_to_dart(expected);
                let _ = writeln!(
                    out,
                    "    expect({string_expr}, startsWith({dart_val}));"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let dart_val = json_to_dart(expected);
                let _ = writeln!(
                    out,
                    "    expect({string_expr}, endsWith({dart_val}));"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "    expect({field_expr}.length, greaterThanOrEqualTo({n}));"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "    expect({field_expr}.length, lessThanOrEqualTo({n}));"
                    );
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "    expect({field_expr}.length, greaterThanOrEqualTo({n}));"
                    );
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "    expect({field_expr}.length, equals({n}));"
                    );
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "    expect({field_expr}, isTrue);");
        }
        "is_false" => {
            let _ = writeln!(out, "    expect({field_expr}, isFalse);");
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let dart_val = json_to_dart(expected);
                let _ = writeln!(
                    out,
                    "    expect({string_expr}, matches({dart_val}));"
                );
            }
        }
        "not_error" => {
            // Already handled by the call succeeding without exception.
        }
        "error" => {
            // Handled at the test case level.
        }
        "method_result" => {
            let _ = writeln!(out, "    // method_result assertions not yet implemented for Dart");
        }
        other => {
            panic!("Dart e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Convert a `serde_json::Value` to a Dart literal string.
fn json_to_dart(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("'{}'", escape_dart(s)),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_dart).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("'{}'", escape_dart(&json_str))
        }
    }
}

/// Escape a string for embedding in a Dart single-quoted string literal.
fn escape_dart(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('$', "\\$")
}
