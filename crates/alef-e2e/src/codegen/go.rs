//! Go e2e test generator using testing.T.

use crate::config::E2eConfig;
use crate::escape::{go_string_literal, sanitize_filename};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, CallbackAction, Fixture, FixtureGroup};
use alef_codegen::naming::go_param_name;
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use alef_core::hash::{self, CommentStyle};
use anyhow::Result;
use heck::ToUpperCamelCase;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// Go e2e code generator.
pub struct GoCodegen;

impl E2eCodegen for GoCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        alef_config: &AlefConfig,
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        // Resolve call config with overrides (for module path and import alias).
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let module_path = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        let import_alias = overrides
            .and_then(|o| o.alias.as_ref())
            .cloned()
            .unwrap_or_else(|| "pkg".to_string());

        // Resolve package config.
        let go_pkg = e2e_config.resolve_package("go");
        let go_module_path = go_pkg
            .as_ref()
            .and_then(|p| p.module.as_ref())
            .cloned()
            .unwrap_or_else(|| module_path.clone());
        let replace_path = go_pkg.as_ref().and_then(|p| p.path.as_ref()).cloned();
        let go_version = go_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                alef_config
                    .resolved_version()
                    .map(|v| format!("v{v}"))
                    .unwrap_or_else(|| "v0.0.0".to_string())
            });
        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
        );

        // Generate go.mod. In registry mode, omit the `replace` directive so the
        // module is fetched from the Go module proxy.
        let effective_replace = match e2e_config.dep_mode {
            crate::config::DependencyMode::Registry => None,
            crate::config::DependencyMode::Local => replace_path.as_deref().map(String::from),
        };
        files.push(GeneratedFile {
            path: output_base.join("go.mod"),
            content: render_go_mod(&go_module_path, effective_replace.as_deref(), &go_version),
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

            let filename = format!("{}_test.go", sanitize_filename(&group.category));
            let content = render_test_file(
                &group.category,
                &active,
                &module_path,
                &import_alias,
                &field_resolver,
                e2e_config,
            );
            files.push(GeneratedFile {
                path: output_base.join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "go"
    }
}

fn render_go_mod(go_module_path: &str, replace_path: Option<&str>, version: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "module e2e_go");
    let _ = writeln!(out);
    let _ = writeln!(out, "go 1.26");
    let _ = writeln!(out);
    let _ = writeln!(out, "require {go_module_path} {version}");

    if let Some(path) = replace_path {
        let _ = writeln!(out);
        let _ = writeln!(out, "replace {go_module_path} => {path}");
    }

    out
}

fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    go_module_path: &str,
    import_alias: &str,
    field_resolver: &FieldResolver,
    e2e_config: &crate::config::E2eConfig,
) -> String {
    let mut out = String::new();

    // Go convention: generated file marker must appear before the package declaration.
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let _ = writeln!(out);

    // Determine if we need the "os" import (mock_url args).
    // Check all resolved per-fixture call args.
    let needs_os = fixtures.iter().any(|f| {
        let call_args = &e2e_config.resolve_call(f.call.as_deref()).args;
        call_args.iter().any(|a| a.arg_type == "mock_url")
    });

    // Determine if we need "encoding/json" (handle args with non-null config).
    let needs_json = fixtures.iter().any(|f| {
        let call_args = &e2e_config.resolve_call(f.call.as_deref()).args;
        call_args.iter().any(|a| a.arg_type == "handle") && {
            call_args.iter().filter(|a| a.arg_type == "handle").any(|a| {
                let v = f.input.get(&a.field).unwrap_or(&serde_json::Value::Null);
                !(v.is_null() || v.is_object() && v.as_object().is_some_and(|o| o.is_empty()))
            })
        }
    });

    // Determine if we need the "fmt" import (CustomTemplate visitor actions with placeholders).
    let needs_fmt = fixtures.iter().any(|f| {
        f.visitor.as_ref().is_some_and(|v| {
            v.callbacks.values().any(|action| {
                if let CallbackAction::CustomTemplate { template } = action {
                    template.contains('{')
                } else {
                    false
                }
            })
        })
    });

    // Determine if we need the "strings" import.
    // Only count assertions whose fields are actually valid for the result type.
    let needs_strings = fixtures.iter().any(|f| {
        f.assertions.iter().any(|a| {
            let type_needs_strings = if a.assertion_type == "equals" {
                // equals with string values needs strings.TrimSpace
                a.value.as_ref().is_some_and(|v| v.is_string())
            } else {
                matches!(
                    a.assertion_type.as_str(),
                    "contains" | "contains_all" | "not_contains" | "starts_with" | "ends_with"
                )
            };
            let field_valid = a
                .field
                .as_ref()
                .map(|f| f.is_empty() || field_resolver.is_valid_for_result(f))
                .unwrap_or(true);
            type_needs_strings && field_valid
        })
    });

    // Determine if we need the testify assert import (used for count_min, count_max,
    // is_true, is_false, and method_result assertions).
    let needs_assert = fixtures.iter().any(|f| {
        f.assertions.iter().any(|a| {
            let field_valid = a
                .field
                .as_ref()
                .map(|f| f.is_empty() || field_resolver.is_valid_for_result(f))
                .unwrap_or(true);
            let type_needs_assert = matches!(
                a.assertion_type.as_str(),
                "count_min"
                    | "count_max"
                    | "is_true"
                    | "is_false"
                    | "method_result"
                    | "min_length"
                    | "max_length"
                    | "matches_regex"
            );
            type_needs_assert && field_valid
        })
    });

    let _ = writeln!(out, "// E2e tests for category: {category}");
    let _ = writeln!(out, "package e2e_test");
    let _ = writeln!(out);
    let _ = writeln!(out, "import (");
    if needs_json {
        let _ = writeln!(out, "\t\"encoding/json\"");
    }
    if needs_fmt {
        let _ = writeln!(out, "\t\"fmt\"");
    }
    if needs_os {
        let _ = writeln!(out, "\t\"os\"");
    }
    if needs_strings {
        let _ = writeln!(out, "\t\"strings\"");
    }
    let _ = writeln!(out, "\t\"testing\"");
    if needs_assert {
        let _ = writeln!(out);
        let _ = writeln!(out, "\t\"github.com/stretchr/testify/assert\"");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "\t{import_alias} \"{go_module_path}\"");
    let _ = writeln!(out, ")");
    let _ = writeln!(out);

    // Emit package-level visitor structs (must be outside any function in Go).
    for fixture in fixtures.iter() {
        if let Some(visitor_spec) = &fixture.visitor {
            let struct_name = visitor_struct_name(&fixture.id);
            emit_go_visitor_struct(&mut out, &struct_name, visitor_spec, import_alias);
            let _ = writeln!(out);
        }
    }

    for (i, fixture) in fixtures.iter().enumerate() {
        render_test_function(&mut out, fixture, import_alias, field_resolver, e2e_config);
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    // Clean up trailing newlines.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn render_test_function(
    out: &mut String,
    fixture: &Fixture,
    import_alias: &str,
    field_resolver: &FieldResolver,
    e2e_config: &crate::config::E2eConfig,
) {
    let fn_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;

    // Resolve call config per-fixture (supports named calls via fixture.call).
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let lang = "go";
    let overrides = call_config.overrides.get(lang);
    let function_name = overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    let result_var = &call_config.result_var;
    let args = &call_config.args;

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let (mut setup_lines, args_str) = build_args_and_setup(&fixture.input, args, import_alias, e2e_config, &fixture.id);

    // Build visitor if present — struct is at package level, just instantiate here.
    let mut visitor_arg = String::new();
    if fixture.visitor.is_some() {
        let struct_name = visitor_struct_name(&fixture.id);
        setup_lines.push(format!("visitor := &{struct_name}{{}}"));
        visitor_arg = "visitor".to_string();
    }

    let final_args = if visitor_arg.is_empty() {
        args_str
    } else {
        format!("{args_str}, {visitor_arg}")
    };

    let _ = writeln!(out, "func Test_{fn_name}(t *testing.T) {{");
    let _ = writeln!(out, "\t// {description}");

    for line in &setup_lines {
        let _ = writeln!(out, "\t{line}");
    }

    if expects_error {
        let _ = writeln!(out, "\t_, err := {import_alias}.{function_name}({final_args})");
        let _ = writeln!(out, "\tif err == nil {{");
        let _ = writeln!(out, "\t\tt.Errorf(\"expected an error, but call succeeded\")");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "}}");
        return;
    }

    // Check if any assertion actually uses the result variable.
    // If all assertions are skipped (field not on result type), use `_` to avoid
    // Go's "declared and not used" compile error.
    let has_usable_assertion = fixture.assertions.iter().any(|a| {
        if a.assertion_type == "not_error" || a.assertion_type == "error" {
            return false;
        }
        // method_result assertions always use the result variable.
        if a.assertion_type == "method_result" {
            return true;
        }
        match &a.field {
            Some(f) if !f.is_empty() => field_resolver.is_valid_for_result(f),
            _ => true,
        }
    });

    let result_binding = if has_usable_assertion {
        result_var.to_string()
    } else {
        "_".to_string()
    };

    // Normal call: check for error assertions first.
    let _ = writeln!(
        out,
        "\t{result_binding}, err := {import_alias}.{function_name}({final_args})"
    );
    let _ = writeln!(out, "\tif err != nil {{");
    let _ = writeln!(out, "\t\tt.Fatalf(\"call failed: %v\", err)");
    let _ = writeln!(out, "\t}}");

    // Collect optional fields referenced by assertions and emit nil-safe
    // dereference blocks so that assertions can use plain string locals.
    // Only dereference fields whose assertion values are strings (or that are
    // used in string-oriented assertions like equals/contains with string values).
    let mut optional_locals: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field {
            if !f.is_empty() {
                let resolved = field_resolver.resolve(f);
                if field_resolver.is_optional(resolved) && !optional_locals.contains_key(f.as_str()) {
                    // Only create deref locals for string-valued fields.
                    // Detect by checking if the assertion value is a string.
                    let is_string_field = assertion.value.as_ref().is_some_and(|v| v.is_string());
                    if !is_string_field {
                        // Non-string optional fields (e.g., *uint64) are handled
                        // by nil guards in render_assertion instead.
                        continue;
                    }
                    let field_expr = field_resolver.accessor(f, "go", result_var);
                    let local_var = go_param_name(&resolved.replace(['.', '[', ']'], "_"));
                    if field_resolver.has_map_access(f) {
                        // Go map access returns a value type (string), not a pointer.
                        // Use the value directly — empty string means not present.
                        let _ = writeln!(out, "\t{local_var} := {field_expr}");
                    } else {
                        let _ = writeln!(out, "\tvar {local_var} string");
                        let _ = writeln!(out, "\tif {field_expr} != nil {{");
                        let _ = writeln!(out, "\t\t{local_var} = *{field_expr}");
                        let _ = writeln!(out, "\t}}");
                    }
                    optional_locals.insert(f.clone(), local_var);
                }
            }
        }
    }

    // Emit assertions, wrapping in nil guards when an intermediate path segment is optional.
    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field {
            if !f.is_empty() && !optional_locals.contains_key(f.as_str()) {
                // Check if any prefix of the dotted path is optional (pointer in Go).
                // e.g., "document.nodes" — if "document" is optional, guard the whole block.
                let parts: Vec<&str> = f.split('.').collect();
                let mut guard_expr: Option<String> = None;
                for i in 1..parts.len() {
                    let prefix = parts[..i].join(".");
                    let resolved_prefix = field_resolver.resolve(&prefix);
                    if field_resolver.is_optional(resolved_prefix) {
                        let accessor = field_resolver.accessor(&prefix, "go", result_var);
                        guard_expr = Some(accessor);
                        break;
                    }
                }
                if let Some(guard) = guard_expr {
                    // Only emit nil guard if the assertion will actually produce code
                    // (not just a skip comment), to avoid empty branches (SA9003).
                    if field_resolver.is_valid_for_result(f) {
                        let _ = writeln!(out, "\tif {guard} != nil {{");
                        // Render into a temporary buffer so we can re-indent by one
                        // tab level to sit inside the nil-guard block.
                        let mut nil_buf = String::new();
                        render_assertion(
                            &mut nil_buf,
                            assertion,
                            result_var,
                            import_alias,
                            field_resolver,
                            &optional_locals,
                        );
                        for line in nil_buf.lines() {
                            let _ = writeln!(out, "\t{line}");
                        }
                        let _ = writeln!(out, "\t}}");
                    } else {
                        render_assertion(
                            out,
                            assertion,
                            result_var,
                            import_alias,
                            field_resolver,
                            &optional_locals,
                        );
                    }
                    continue;
                }
            }
        }
        render_assertion(
            out,
            assertion,
            result_var,
            import_alias,
            field_resolver,
            &optional_locals,
        );
    }

    let _ = writeln!(out, "}}");
}

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// Returns `(setup_lines, args_string)`.
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    import_alias: &str,
    e2e_config: &crate::config::E2eConfig,
    fixture_id: &str,
) -> (Vec<String>, String) {
    use heck::ToUpperCamelCase;

    if args.is_empty() {
        return (Vec::new(), json_to_go(input));
    }

    let overrides = e2e_config.call.overrides.get("go");
    let options_type = overrides.and_then(|o| o.options_type.as_deref());

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "mock_url" {
            setup_lines.push(format!(
                "{} := os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\"",
                arg.name,
            ));
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "handle" {
            // Generate a CreateEngine (or equivalent) call and pass the variable.
            let constructor_name = format!("Create{}", arg.name.to_upper_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!(
                    "{name}, createErr := {import_alias}.{constructor_name}(nil)\n\tif createErr != nil {{\n\t\tt.Fatalf(\"create handle failed: %v\", createErr)\n\t}}",
                    name = arg.name,
                ));
            } else {
                let json_str = serde_json::to_string(config_value).unwrap_or_default();
                let go_literal = go_string_literal(&json_str);
                let name = &arg.name;
                setup_lines.push(format!(
                    "var {name}Config {import_alias}.CrawlConfig\n\tif err := json.Unmarshal([]byte({go_literal}), &{name}Config); err != nil {{\n\t\tt.Fatalf(\"config parse failed: %v\", err)\n\t}}"
                ));
                setup_lines.push(format!(
                    "{name}, createErr := {import_alias}.{constructor_name}(&{name}Config)\n\tif createErr != nil {{\n\t\tt.Fatalf(\"create handle failed: %v\", createErr)\n\t}}"
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
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "nil".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                // For json_object args with options_type: construct using functional options.
                if let (Some(opts_type), "json_object") = (options_type, arg.arg_type.as_str()) {
                    if let Some(obj) = v.as_object() {
                        let with_calls: Vec<String> = obj
                            .iter()
                            .map(|(k, vv)| {
                                let func_name = format!("With{}{}", opts_type, k.to_upper_camel_case());
                                let go_val = json_to_go(vv);
                                format!("htmd.{func_name}({go_val})")
                            })
                            .collect();
                        let new_fn = format!("New{opts_type}");
                        parts.push(format!("htmd.{new_fn}({})", with_calls.join(", ")));
                        continue;
                    }
                }
                parts.push(json_to_go(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
}

fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    import_alias: &str,
    field_resolver: &FieldResolver,
    optional_locals: &std::collections::HashMap<String, String>,
) {
    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "\t// skipped: field '{f}' not available on result type");
            return;
        }
    }

    let field_expr = match &assertion.field {
        Some(f) if !f.is_empty() => {
            // Use the local variable if the field was dereferenced above.
            if let Some(local_var) = optional_locals.get(f.as_str()) {
                local_var.clone()
            } else {
                field_resolver.accessor(f, "go", result_var)
            }
        }
        _ => result_var.to_string(),
    };

    // Check if the field (after resolution) is optional, which means it's a pointer in Go.
    // Also check if a `.length` suffix's parent is optional (e.g., metadata.headings.length
    // where metadata.headings is optional → len() needs dereference).
    let is_optional = assertion
        .field
        .as_ref()
        .map(|f| {
            let resolved = field_resolver.resolve(f);
            let check_path = resolved
                .strip_suffix(".length")
                .or_else(|| resolved.strip_suffix(".count"))
                .or_else(|| resolved.strip_suffix(".size"))
                .unwrap_or(resolved);
            field_resolver.is_optional(check_path) && !optional_locals.contains_key(f.as_str())
        })
        .unwrap_or(false);

    // When field_expr is `len(X)` and X is an optional (pointer) field, rewrite to `len(*X)`
    // and we'll wrap with a nil guard in the assertion handlers.
    let field_expr = if is_optional && field_expr.starts_with("len(") && field_expr.ends_with(')') {
        let inner = &field_expr[4..field_expr.len() - 1];
        format!("len(*{inner})")
    } else {
        field_expr
    };
    // Build the nil-guard expression for the inner pointer (without len wrapper).
    let nil_guard_expr = if is_optional && field_expr.starts_with("len(*") {
        Some(field_expr[5..field_expr.len() - 1].to_string())
    } else {
        None
    };

    // For optional non-string fields that weren't dereferenced into locals,
    // we need to dereference the pointer in comparisons.
    let deref_field_expr = if is_optional && !field_expr.starts_with("len(") {
        format!("*{field_expr}")
    } else {
        field_expr.clone()
    };

    // Detect array element access (e.g., `result.Assets[0].ContentHash`).
    // When the field_expr contains `[0]`, we must guard against an out-of-bounds
    // panic by checking that the array is non-empty first.
    // Extract the array slice expression (everything before `[0]`).
    let array_guard: Option<String> = if let Some(idx) = field_expr.find("[0]") {
        let array_expr = &field_expr[..idx];
        Some(array_expr.to_string())
    } else {
        None
    };

    // Render the assertion into a temporary buffer first, then wrap with the array
    // bounds guard (if needed) by adding one extra level of indentation.
    let mut assertion_buf = String::new();
    let out_ref = &mut assertion_buf;

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                // For string equality, trim whitespace to handle trailing newlines from the converter.
                if expected.is_string() {
                    // Wrap field expression with strings.TrimSpace() for string comparisons.
                    let trimmed_field = if is_optional && !field_expr.starts_with("len(") {
                        format!("strings.TrimSpace(*{field_expr})")
                    } else {
                        format!("strings.TrimSpace({field_expr})")
                    };
                    if is_optional && !field_expr.starts_with("len(") {
                        let _ = writeln!(out_ref, "\tif {field_expr} != nil && {trimmed_field} != {go_val} {{");
                    } else {
                        let _ = writeln!(out_ref, "\tif {trimmed_field} != {go_val} {{");
                    }
                } else if is_optional && !field_expr.starts_with("len(") {
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil && {deref_field_expr} != {go_val} {{");
                } else {
                    let _ = writeln!(out_ref, "\tif {field_expr} != {go_val} {{");
                }
                let _ = writeln!(out_ref, "\t\tt.Errorf(\"equals mismatch: got %v\", {field_expr})");
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                let field_for_contains = if is_optional
                    && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()))
                {
                    format!("string(*{field_expr})")
                } else {
                    format!("string({field_expr})")
                };
                let _ = writeln!(out_ref, "\tif !strings.Contains({field_for_contains}, {go_val}) {{");
                let _ = writeln!(
                    out_ref,
                    "\t\tt.Errorf(\"expected to contain %s, got %v\", {go_val}, {field_expr})"
                );
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let go_val = json_to_go(val);
                    let field_for_contains = if is_optional
                        && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()))
                    {
                        format!("string(*{field_expr})")
                    } else {
                        format!("string({field_expr})")
                    };
                    let _ = writeln!(out_ref, "\tif !strings.Contains({field_for_contains}, {go_val}) {{");
                    let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected to contain %s\", {go_val})");
                    let _ = writeln!(out_ref, "\t}}");
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                let field_for_contains = if is_optional
                    && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()))
                {
                    format!("string(*{field_expr})")
                } else {
                    format!("string({field_expr})")
                };
                let _ = writeln!(out_ref, "\tif strings.Contains({field_for_contains}, {go_val}) {{");
                let _ = writeln!(
                    out_ref,
                    "\t\tt.Errorf(\"expected NOT to contain %s, got %v\", {go_val}, {field_expr})"
                );
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "not_empty" => {
            if is_optional {
                let _ = writeln!(out_ref, "\tif {field_expr} == nil || len(*{field_expr}) == 0 {{");
            } else {
                let _ = writeln!(out_ref, "\tif len({field_expr}) == 0 {{");
            }
            let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected non-empty value\")");
            let _ = writeln!(out_ref, "\t}}");
        }
        "is_empty" => {
            if is_optional {
                let _ = writeln!(out_ref, "\tif {field_expr} != nil && len(*{field_expr}) != 0 {{");
            } else {
                let _ = writeln!(out_ref, "\tif len({field_expr}) != 0 {{");
            }
            let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected empty value, got %v\", {field_expr})");
            let _ = writeln!(out_ref, "\t}}");
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let field_for_contains = if is_optional
                    && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()))
                {
                    format!("*{field_expr}")
                } else {
                    field_expr.clone()
                };
                let _ = writeln!(out_ref, "\t{{");
                let _ = writeln!(out_ref, "\t\tfound := false");
                for val in values {
                    let go_val = json_to_go(val);
                    let _ = writeln!(
                        out_ref,
                        "\t\tif strings.Contains({field_for_contains}, {go_val}) {{ found = true }}"
                    );
                }
                let _ = writeln!(out_ref, "\t\tif !found {{");
                let _ = writeln!(
                    out_ref,
                    "\t\t\tt.Errorf(\"expected to contain at least one of the specified values\")"
                );
                let _ = writeln!(out_ref, "\t\t}}");
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let go_val = json_to_go(val);
                // Use `< N+1` instead of `<= N` to avoid golangci-lint sloppyLen
                // warning when N is 0 (len(x) <= 0 → len(x) < 1).
                if let Some(n) = val.as_u64() {
                    let next = n + 1;
                    let _ = writeln!(out_ref, "\tif {field_expr} < {next} {{");
                } else {
                    let _ = writeln!(out_ref, "\tif {field_expr} <= {go_val} {{");
                }
                let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected > {go_val}, got %v\", {field_expr})");
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let go_val = json_to_go(val);
                let _ = writeln!(out_ref, "\tif {field_expr} >= {go_val} {{");
                let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected < {go_val}, got %v\", {field_expr})");
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let go_val = json_to_go(val);
                if let Some(ref guard) = nil_guard_expr {
                    let _ = writeln!(out_ref, "\tif {guard} != nil {{");
                    let _ = writeln!(out_ref, "\t\tif {field_expr} < {go_val} {{");
                    let _ = writeln!(
                        out_ref,
                        "\t\t\tt.Errorf(\"expected >= {go_val}, got %v\", {field_expr})"
                    );
                    let _ = writeln!(out_ref, "\t\t}}");
                    let _ = writeln!(out_ref, "\t}}");
                } else {
                    let _ = writeln!(out_ref, "\tif {field_expr} < {go_val} {{");
                    let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected >= {go_val}, got %v\", {field_expr})");
                    let _ = writeln!(out_ref, "\t}}");
                }
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let go_val = json_to_go(val);
                let _ = writeln!(out_ref, "\tif {field_expr} > {go_val} {{");
                let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected <= {go_val}, got %v\", {field_expr})");
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                let field_for_prefix = if is_optional
                    && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()))
                {
                    format!("string(*{field_expr})")
                } else {
                    format!("string({field_expr})")
                };
                let _ = writeln!(out_ref, "\tif !strings.HasPrefix({field_for_prefix}, {go_val}) {{");
                let _ = writeln!(
                    out_ref,
                    "\t\tt.Errorf(\"expected to start with %s, got %v\", {go_val}, {field_expr})"
                );
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if is_optional {
                        let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                        let _ = writeln!(
                            out_ref,
                            "\t\tassert.GreaterOrEqual(t, len(*{field_expr}), {n}, \"expected at least {n} elements\")"
                        );
                        let _ = writeln!(out_ref, "\t}}");
                    } else {
                        let _ = writeln!(
                            out_ref,
                            "\tassert.GreaterOrEqual(t, len({field_expr}), {n}, \"expected at least {n} elements\")"
                        );
                    }
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if is_optional {
                        let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                        let _ = writeln!(
                            out_ref,
                            "\t\tassert.Equal(t, len(*{field_expr}), {n}, \"expected exactly {n} elements\")"
                        );
                        let _ = writeln!(out_ref, "\t}}");
                    } else {
                        let _ = writeln!(
                            out_ref,
                            "\tassert.Equal(t, len({field_expr}), {n}, \"expected exactly {n} elements\")"
                        );
                    }
                }
            }
        }
        "is_true" => {
            if is_optional {
                let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                let _ = writeln!(out_ref, "\t\tassert.True(t, *{field_expr}, \"expected true\")");
                let _ = writeln!(out_ref, "\t}}");
            } else {
                let _ = writeln!(out_ref, "\tassert.True(t, {field_expr}, \"expected true\")");
            }
        }
        "is_false" => {
            if is_optional {
                let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                let _ = writeln!(out_ref, "\t\tassert.False(t, *{field_expr}, \"expected false\")");
                let _ = writeln!(out_ref, "\t}}");
            } else {
                let _ = writeln!(out_ref, "\tassert.False(t, {field_expr}, \"expected false\")");
            }
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                let info = build_go_method_call(result_var, method_name, assertion.args.as_ref(), import_alias);
                let check = assertion.check.as_deref().unwrap_or("is_true");
                // For pointer-returning functions, dereference with `*`. Value-returning
                // functions (e.g., NodeInfo field access) are used directly.
                let deref_expr = if info.is_pointer {
                    format!("*{}", info.call_expr)
                } else {
                    info.call_expr.clone()
                };
                match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            if val.is_boolean() {
                                if val.as_bool() == Some(true) {
                                    let _ = writeln!(out_ref, "\tassert.True(t, {deref_expr}, \"expected true\")");
                                } else {
                                    let _ = writeln!(out_ref, "\tassert.False(t, {deref_expr}, \"expected false\")");
                                }
                            } else {
                                // Apply type cast to numeric literals when the method returns
                                // a typed uint (e.g., *uint) to avoid reflect.DeepEqual
                                // mismatches between int and uint in testify's assert.Equal.
                                let go_val = if let Some(cast) = info.value_cast {
                                    if val.is_number() {
                                        format!("{cast}({})", json_to_go(val))
                                    } else {
                                        json_to_go(val)
                                    }
                                } else {
                                    json_to_go(val)
                                };
                                let _ = writeln!(
                                    out_ref,
                                    "\tassert.Equal(t, {go_val}, {deref_expr}, \"method_result equals assertion failed\")"
                                );
                            }
                        }
                    }
                    "is_true" => {
                        let _ = writeln!(out_ref, "\tassert.True(t, {deref_expr}, \"expected true\")");
                    }
                    "is_false" => {
                        let _ = writeln!(out_ref, "\tassert.False(t, {deref_expr}, \"expected false\")");
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            // Use the value_cast type if available (e.g., uint for named_children_count).
                            let cast = info.value_cast.unwrap_or("uint");
                            let _ = writeln!(
                                out_ref,
                                "\tassert.GreaterOrEqual(t, {deref_expr}, {cast}({n}), \"expected >= {n}\")"
                            );
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(
                                out_ref,
                                "\tassert.GreaterOrEqual(t, len({deref_expr}), {n}, \"expected at least {n} elements\")"
                            );
                        }
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            let go_val = json_to_go(val);
                            let _ = writeln!(
                                out_ref,
                                "\tassert.Contains(t, {deref_expr}, {go_val}, \"expected result to contain value\")"
                            );
                        }
                    }
                    "is_error" => {
                        let _ = writeln!(out_ref, "\t{{");
                        let _ = writeln!(out_ref, "\t\t_, methodErr := {}", info.call_expr);
                        let _ = writeln!(out_ref, "\t\tassert.Error(t, methodErr)");
                        let _ = writeln!(out_ref, "\t}}");
                    }
                    other_check => {
                        panic!("Go e2e generator: unsupported method_result check type: {other_check}");
                    }
                }
            } else {
                panic!("Go e2e generator: method_result assertion missing 'method' field");
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if is_optional {
                        let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                        let _ = writeln!(
                            out_ref,
                            "\t\tassert.GreaterOrEqual(t, len(*{field_expr}), {n}, \"expected length >= {n}\")"
                        );
                        let _ = writeln!(out_ref, "\t}}");
                    } else {
                        let _ = writeln!(
                            out_ref,
                            "\tassert.GreaterOrEqual(t, len({field_expr}), {n}, \"expected length >= {n}\")"
                        );
                    }
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if is_optional {
                        let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                        let _ = writeln!(
                            out_ref,
                            "\t\tassert.LessOrEqual(t, len(*{field_expr}), {n}, \"expected length <= {n}\")"
                        );
                        let _ = writeln!(out_ref, "\t}}");
                    } else {
                        let _ = writeln!(
                            out_ref,
                            "\tassert.LessOrEqual(t, len({field_expr}), {n}, \"expected length <= {n}\")"
                        );
                    }
                }
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                let field_for_suffix = if is_optional
                    && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()))
                {
                    format!("string(*{field_expr})")
                } else {
                    format!("string({field_expr})")
                };
                let _ = writeln!(out_ref, "\tif !strings.HasSuffix({field_for_suffix}, {go_val}) {{");
                let _ = writeln!(
                    out_ref,
                    "\t\tt.Errorf(\"expected to end with %s, got %v\", {go_val}, {field_expr})"
                );
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                let field_for_regex = if is_optional
                    && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()))
                {
                    format!("*{field_expr}")
                } else {
                    field_expr.clone()
                };
                let _ = writeln!(
                    out_ref,
                    "\tassert.Regexp(t, {go_val}, {field_for_regex}, \"expected value to match regex\")"
                );
            }
        }
        "not_error" => {
            // Already handled by the `if err != nil` check above.
        }
        "error" => {
            // Handled at the test function level.
        }
        other => {
            panic!("Go e2e generator: unsupported assertion type: {other}");
        }
    }

    // If the assertion accesses an array element via [0], wrap the generated code in a
    // bounds check to prevent an index-out-of-range panic when the array is empty.
    if let Some(ref arr) = array_guard {
        if !assertion_buf.is_empty() {
            let _ = writeln!(out, "\tif len({arr}) > 0 {{");
            // Re-indent each line by one additional tab level.
            for line in assertion_buf.lines() {
                let _ = writeln!(out, "\t{line}");
            }
            let _ = writeln!(out, "\t}}");
        }
    } else {
        out.push_str(&assertion_buf);
    }
}

/// Metadata about the return type of a Go method call for `method_result` assertions.
struct GoMethodCallInfo {
    /// The call expression string.
    call_expr: String,
    /// Whether the return type is a pointer (needs `*` dereference for value comparison).
    is_pointer: bool,
    /// Optional Go type cast to apply to numeric literal values in `equals` assertions
    /// (e.g., `"uint"` so that `0` becomes `uint(0)` to match `*uint` deref type).
    value_cast: Option<&'static str>,
}

/// Build a Go call expression for a `method_result` assertion on a tree-sitter Tree.
///
/// Maps method names to the appropriate Go function calls, matching the Go binding API
/// in `packages/go/binding.go`. Returns a [`GoMethodCallInfo`] describing the call and
/// its return type characteristics.
///
/// Return types by method:
/// - `has_error_nodes`, `contains_node_type` → `*bool` (pointer)
/// - `error_count` → `*uint` (pointer, value_cast = "uint")
/// - `tree_to_sexp` → `*string` (pointer)
/// - `root_node_type` → `string` via `RootNodeInfo(tree).Kind` (value)
/// - `named_children_count` → `uint` via `RootNodeInfo(tree).NamedChildCount` (value, value_cast = "uint")
/// - `find_nodes_by_type` → `*[]NodeInfo` (pointer to slice)
/// - `run_query` → `(*[]QueryMatch, error)` (pointer + error; use `is_error` check type)
fn build_go_method_call(
    result_var: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
    import_alias: &str,
) -> GoMethodCallInfo {
    match method_name {
        "root_node_type" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.RootNodeInfo({result_var}).Kind"),
            is_pointer: false,
            value_cast: None,
        },
        "named_children_count" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.RootNodeInfo({result_var}).NamedChildCount"),
            is_pointer: false,
            value_cast: Some("uint"),
        },
        "has_error_nodes" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.TreeHasErrorNodes({result_var})"),
            is_pointer: true,
            value_cast: None,
        },
        "error_count" | "tree_error_count" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.TreeErrorCount({result_var})"),
            is_pointer: true,
            value_cast: Some("uint"),
        },
        "tree_to_sexp" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.TreeToSexp({result_var})"),
            is_pointer: true,
            value_cast: None,
        },
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            GoMethodCallInfo {
                call_expr: format!("{import_alias}.TreeContainsNodeType({result_var}, \"{node_type}\")"),
                is_pointer: true,
                value_cast: None,
            }
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            GoMethodCallInfo {
                call_expr: format!("{import_alias}.FindNodesByType({result_var}, \"{node_type}\")"),
                is_pointer: true,
                value_cast: None,
            }
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
            let query_lit = go_string_literal(query_source);
            let lang_lit = go_string_literal(language);
            // RunQuery returns (*[]QueryMatch, error) — use is_error check type.
            GoMethodCallInfo {
                call_expr: format!("{import_alias}.RunQuery({result_var}, {lang_lit}, {query_lit}, []byte(source))"),
                is_pointer: false,
                value_cast: None,
            }
        }
        other => {
            let method_pascal = other.to_upper_camel_case();
            GoMethodCallInfo {
                call_expr: format!("{result_var}.{method_pascal}()"),
                is_pointer: false,
                value_cast: None,
            }
        }
    }
}

/// Convert a `serde_json::Value` to a Go literal string.
fn json_to_go(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => go_string_literal(s),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "nil".to_string(),
        // For complex types, serialize to JSON string and pass as literal.
        other => go_string_literal(&other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Visitor generation
// ---------------------------------------------------------------------------

/// Derive a unique, exported Go struct name for a visitor from a fixture ID.
///
/// E.g. `visitor_continue_default` → `visitorContinueDefault` (unexported, avoids
/// polluting the exported API of the test package while still being package-level).
fn visitor_struct_name(fixture_id: &str) -> String {
    use heck::ToUpperCamelCase;
    // Use UpperCamelCase so Go treats it as exported — required for method sets.
    format!("testVisitor{}", fixture_id.to_upper_camel_case())
}

/// Emit a package-level Go struct declaration and all its visitor methods.
fn emit_go_visitor_struct(
    out: &mut String,
    struct_name: &str,
    visitor_spec: &crate::fixture::VisitorSpec,
    import_alias: &str,
) {
    let _ = writeln!(out, "type {struct_name} struct{{}}");
    for (method_name, action) in &visitor_spec.callbacks {
        emit_go_visitor_method(out, struct_name, method_name, action, import_alias);
    }
}

/// Emit a Go visitor method for a callback action on the named struct.
fn emit_go_visitor_method(
    out: &mut String,
    struct_name: &str,
    method_name: &str,
    action: &CallbackAction,
    import_alias: &str,
) {
    let camel_method = method_to_camel(method_name);
    let params = match method_name {
        "visit_link" => format!("_ {import_alias}.NodeContext, href, text, title string"),
        "visit_image" => format!("_ {import_alias}.NodeContext, src, alt, title string"),
        "visit_heading" => format!("_ {import_alias}.NodeContext, level int, text, id string"),
        "visit_code_block" => format!("_ {import_alias}.NodeContext, lang, code string"),
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
        | "visit_definition_description" => format!("_ {import_alias}.NodeContext, text string"),
        "visit_text" => format!("_ {import_alias}.NodeContext, text string"),
        "visit_list_item" => {
            format!("_ {import_alias}.NodeContext, ordered bool, marker, text string")
        }
        "visit_blockquote" => format!("_ {import_alias}.NodeContext, content string, depth int"),
        "visit_table_row" => format!("_ {import_alias}.NodeContext, cells []string, isHeader bool"),
        "visit_custom_element" => format!("_ {import_alias}.NodeContext, tagName, html string"),
        "visit_form" => format!("_ {import_alias}.NodeContext, actionUrl, method string"),
        "visit_input" => format!("_ {import_alias}.NodeContext, inputType, name, value string"),
        "visit_audio" | "visit_video" | "visit_iframe" => {
            format!("_ {import_alias}.NodeContext, src string")
        }
        "visit_details" => format!("_ {import_alias}.NodeContext, isOpen bool"),
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => {
            format!("_ {import_alias}.NodeContext, output string")
        }
        "visit_list_start" => format!("_ {import_alias}.NodeContext, ordered bool"),
        "visit_list_end" => format!("_ {import_alias}.NodeContext, ordered bool, output string"),
        _ => format!("_ {import_alias}.NodeContext"),
    };

    let _ = writeln!(
        out,
        "func (v *{struct_name}) {camel_method}({params}) {import_alias}.VisitResult {{"
    );
    match action {
        CallbackAction::Skip => {
            let _ = writeln!(out, "\treturn {import_alias}.VisitResultSkip");
        }
        CallbackAction::Continue => {
            let _ = writeln!(out, "\treturn {import_alias}.VisitResultContinue");
        }
        CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "\treturn {import_alias}.VisitResultPreserveHtml");
        }
        CallbackAction::Custom { output } => {
            let escaped = go_string_literal(output);
            let _ = writeln!(out, "\treturn {import_alias}.VisitResultCustom({escaped})");
        }
        CallbackAction::CustomTemplate { template } => {
            // Convert {var} placeholders to %s format verbs and collect arg names.
            // E.g. `QUOTE: "{text}"` → fmt.Sprintf("QUOTE: \"%s\"", text)
            let (fmt_str, fmt_args) = template_to_sprintf(template);
            let escaped_fmt = go_string_literal(&fmt_str);
            if fmt_args.is_empty() {
                let _ = writeln!(out, "\treturn {import_alias}.VisitResultCustom({escaped_fmt})");
            } else {
                let args_str = fmt_args.join(", ");
                let _ = writeln!(
                    out,
                    "\treturn {import_alias}.VisitResultCustom(fmt.Sprintf({escaped_fmt}, {args_str}))"
                );
            }
        }
    }
    let _ = writeln!(out, "}}");
}

/// Convert a `{var}` template string into a `fmt.Sprintf` format string and argument list.
///
/// For example, `QUOTE: "{text}"` becomes `("QUOTE: \"%s\"", vec!["text"])`.
fn template_to_sprintf(template: &str) -> (String, Vec<String>) {
    let mut fmt_str = String::new();
    let mut args: Vec<String> = Vec::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            // Collect placeholder name until '}'.
            let mut name = String::new();
            for inner in chars.by_ref() {
                if inner == '}' {
                    break;
                }
                name.push(inner);
            }
            fmt_str.push_str("%s");
            args.push(name);
        } else {
            fmt_str.push(c);
        }
    }
    (fmt_str, args)
}

/// Convert snake_case method names to Go camelCase.
fn method_to_camel(snake: &str) -> String {
    use heck::ToUpperCamelCase;
    snake.to_upper_camel_case()
}
