//! Rust e2e test code generator.
//!
//! Generates `e2e/rust/Cargo.toml` and `tests/{category}_test.rs` files from
//! JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

use crate::config::E2eConfig;
use crate::escape::{escape_rust, rust_raw_string, sanitize_filename, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use anyhow::Result;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

/// Rust e2e test code generator.
pub struct RustE2eCodegen;

impl super::E2eCodegen for RustE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        alef_config: &AlefConfig,
    ) -> Result<Vec<GeneratedFile>> {
        let mut files = Vec::new();
        let output_base = PathBuf::from(e2e_config.effective_output()).join("rust");

        // Resolve crate name and path from config.
        let crate_name = resolve_crate_name(e2e_config, alef_config);
        let crate_path = resolve_crate_path(e2e_config, &crate_name);
        let dep_name = crate_name.replace('-', "_");

        // Cargo.toml
        // Check if any fixture uses json_object args (needs serde_json dep).
        let needs_serde_json = e2e_config
            .call
            .args
            .iter()
            .any(|a| a.arg_type == "json_object" || a.arg_type == "handle");
        let crate_version = resolve_crate_version(e2e_config);
        files.push(GeneratedFile {
            path: output_base.join("Cargo.toml"),
            content: render_cargo_toml(
                &crate_name,
                &dep_name,
                &crate_path,
                needs_serde_json,
                e2e_config.dep_mode,
                crate_version.as_deref(),
            ),
            generated_header: true,
        });

        // Per-category test files.
        for group in groups {
            let fixtures: Vec<&Fixture> = group.fixtures.iter().filter(|f| !is_skipped(f, "rust")).collect();

            if fixtures.is_empty() {
                continue;
            }

            let filename = format!("{}_test.rs", sanitize_filename(&group.category));
            let content = render_test_file(&group.category, &fixtures, e2e_config, &dep_name);

            files.push(GeneratedFile {
                path: output_base.join("tests").join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "rust"
    }
}

// ---------------------------------------------------------------------------
// Config resolution helpers
// ---------------------------------------------------------------------------

fn resolve_crate_name(_e2e_config: &E2eConfig, alef_config: &AlefConfig) -> String {
    // Always use the Cargo package name (with hyphens) from alef.toml [crate].
    // The `crate_name` override in [e2e.call.overrides.rust] is for the Rust
    // import identifier, not the Cargo package name.
    alef_config.crate_config.name.clone()
}

fn resolve_crate_path(e2e_config: &E2eConfig, crate_name: &str) -> String {
    e2e_config
        .resolve_package("rust")
        .and_then(|p| p.path.clone())
        .unwrap_or_else(|| format!("../../crates/{crate_name}"))
}

fn resolve_crate_version(e2e_config: &E2eConfig) -> Option<String> {
    e2e_config.resolve_package("rust").and_then(|p| p.version.clone())
}

fn resolve_function_name(e2e_config: &E2eConfig) -> String {
    e2e_config
        .call
        .overrides
        .get("rust")
        .and_then(|o| o.function.clone())
        .unwrap_or_else(|| e2e_config.call.function.clone())
}

fn resolve_module(e2e_config: &E2eConfig, dep_name: &str) -> String {
    // For Rust, the module name is the crate identifier (underscores).
    // Priority: override.crate_name > override.module > dep_name
    let overrides = e2e_config.call.overrides.get("rust");
    overrides
        .and_then(|o| o.crate_name.clone())
        .or_else(|| overrides.and_then(|o| o.module.clone()))
        .unwrap_or_else(|| dep_name.to_string())
}

fn is_skipped(fixture: &Fixture, language: &str) -> bool {
    fixture.skip.as_ref().is_some_and(|s| s.should_skip(language))
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_cargo_toml(
    crate_name: &str,
    dep_name: &str,
    crate_path: &str,
    needs_serde_json: bool,
    dep_mode: crate::config::DependencyMode,
    version: Option<&str>,
) -> String {
    let e2e_name = format!("{dep_name}-e2e-rust");
    let dep_spec = match dep_mode {
        crate::config::DependencyMode::Registry => {
            let ver = version.unwrap_or("0.1.0");
            if crate_name != dep_name {
                format!("{dep_name} = {{ package = \"{crate_name}\", version = \"{ver}\" }}")
            } else {
                format!("{dep_name} = \"{ver}\"")
            }
        }
        crate::config::DependencyMode::Local => {
            // When the crate name has hyphens, Cargo needs `package = "name-with-hyphens"`
            // because the dep key uses underscores (Rust identifier).
            if crate_name != dep_name {
                format!("{dep_name} = {{ package = \"{crate_name}\", path = \"{crate_path}\" }}")
            } else {
                format!("{dep_name} = {{ path = \"{crate_path}\" }}")
            }
        }
    };
    let serde_line = if needs_serde_json { "\nserde_json = \"1\"" } else { "" };
    // When using registry mode the generated Cargo.toml lives inside a directory
    // that may be auto-discovered as part of a parent Cargo workspace.  Adding an
    // empty [workspace] table tells Cargo that this crate is its own standalone
    // workspace and opts out of any parent workspace discovery.
    let workspace_section = match dep_mode {
        crate::config::DependencyMode::Registry => "\n[workspace]\n",
        crate::config::DependencyMode::Local => "",
    };
    let machete_section = if needs_serde_json {
        "\n[package.metadata.cargo-machete]\nignored = [\"serde_json\"]\n"
    } else {
        ""
    };
    format!(
        r#"# This file is auto-generated by alef. DO NOT EDIT.
{workspace_section}
[package]
name = "{e2e_name}"
version = "0.1.0"
edition = "2021"
license = "MIT"
publish = false

[dependencies]
{dep_spec}{serde_line}
tokio = {{ version = "1", features = ["full"] }}
{machete_section}"#
    )
}

fn render_test_file(category: &str, fixtures: &[&Fixture], e2e_config: &E2eConfig, dep_name: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// This file is auto-generated by alef. DO NOT EDIT.");
    let _ = writeln!(out, "//! E2e tests for category: {category}");
    let _ = writeln!(out);

    let module = resolve_module(e2e_config, dep_name);
    let function_name = resolve_function_name(e2e_config);
    let field_resolver = FieldResolver::new(
        &e2e_config.fields,
        &e2e_config.fields_optional,
        &e2e_config.result_fields,
        &e2e_config.fields_array,
    );

    let _ = writeln!(out, "use {module}::{function_name};");

    // Import handle constructor functions and the config type they use.
    let has_handle_args = e2e_config.call.args.iter().any(|a| a.arg_type == "handle");
    if has_handle_args {
        let _ = writeln!(out, "use {module}::CrawlConfig;");
    }
    for arg in &e2e_config.call.args {
        if arg.arg_type == "handle" {
            use heck::ToSnakeCase;
            let constructor_name = format!("create_{}", arg.name.to_snake_case());
            let _ = writeln!(out, "use {module}::{constructor_name};");
        }
    }

    let _ = writeln!(out);

    for fixture in fixtures {
        render_test_function(&mut out, fixture, e2e_config, dep_name, &field_resolver);
        let _ = writeln!(out);
    }

    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn render_test_function(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    dep_name: &str,
    field_resolver: &FieldResolver,
) {
    let fn_name = sanitize_ident(&fixture.id);
    let description = &fixture.description;
    let function_name = resolve_function_name(e2e_config);
    let module = resolve_module(e2e_config, dep_name);
    let result_var = &e2e_config.call.result_var;

    let is_async = e2e_config.call.r#async;
    if is_async {
        let _ = writeln!(out, "#[tokio::test]");
        let _ = writeln!(out, "async fn test_{fn_name}() {{");
    } else {
        let _ = writeln!(out, "#[test]");
        let _ = writeln!(out, "fn test_{fn_name}() {{");
    }
    let _ = writeln!(out, "    // {description}");

    // Check if any assertion is an error assertion.
    let has_error_assertion = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Emit input variable bindings from args config.
    let mut arg_exprs: Vec<String> = Vec::new();
    for arg in &e2e_config.call.args {
        let value = resolve_field(&fixture.input, &arg.field);
        let var_name = &arg.name;
        let (bindings, expr) = render_rust_arg(var_name, value, &arg.arg_type, arg.optional, &module, &fixture.id);
        for binding in &bindings {
            let _ = writeln!(out, "    {binding}");
        }
        arg_exprs.push(expr);
    }

    let args_str = arg_exprs.join(", ");

    let await_suffix = if is_async { ".await" } else { "" };

    if has_error_assertion {
        let _ = writeln!(out, "    let {result_var} = {function_name}({args_str}){await_suffix};");
        // Render error assertions.
        for assertion in &fixture.assertions {
            render_assertion(out, assertion, result_var, dep_name, true, &[], field_resolver);
        }
        let _ = writeln!(out, "}}");
        return;
    }

    // Non-error path: unwrap the result.
    let has_not_error = fixture.assertions.iter().any(|a| a.assertion_type == "not_error");

    // Check if any assertion actually uses the result variable.
    // If all assertions are skipped (field not on result type), use `_` to avoid
    // Rust's "variable never used" warning.
    let has_usable_assertion = fixture.assertions.iter().any(|a| {
        if a.assertion_type == "not_error" || a.assertion_type == "error" {
            return false;
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

    if has_not_error || !fixture.assertions.is_empty() {
        let _ = writeln!(
            out,
            "    let {result_binding} = {function_name}({args_str}){await_suffix}.expect(\"should succeed\");"
        );
    } else {
        let _ = writeln!(
            out,
            "    let {result_binding} = {function_name}({args_str}){await_suffix};"
        );
    }

    // Emit Option field unwrap bindings for any fields accessed in assertions.
    // Use FieldResolver to handle optional fields, including nested/aliased paths.
    let string_assertion_types = [
        "equals",
        "contains",
        "contains_all",
        "contains_any",
        "not_contains",
        "starts_with",
        "ends_with",
        "min_length",
        "max_length",
        "matches_regex",
    ];
    let mut unwrapped_fields: Vec<(String, String)> = Vec::new(); // (fixture_field, local_var)
    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field {
            if !f.is_empty()
                && string_assertion_types.contains(&assertion.assertion_type.as_str())
                && !unwrapped_fields.iter().any(|(ff, _)| ff == f)
            {
                // Only unwrap optional string fields — numeric optionals (u64, usize)
                // don't support .as_deref() and should be compared directly.
                let is_string_assertion = assertion.value.as_ref().is_none_or(|v| v.is_string());
                if !is_string_assertion {
                    continue;
                }
                if let Some((binding, local_var)) = field_resolver.rust_unwrap_binding(f, result_var) {
                    let _ = writeln!(out, "    {binding}");
                    unwrapped_fields.push((f.clone(), local_var));
                }
            }
        }
    }

    // Render assertions.
    for assertion in &fixture.assertions {
        if assertion.assertion_type == "not_error" {
            // Already handled by .expect() above.
            continue;
        }
        render_assertion(
            out,
            assertion,
            result_var,
            dep_name,
            false,
            &unwrapped_fields,
            field_resolver,
        );
    }

    let _ = writeln!(out, "}}");
}

// ---------------------------------------------------------------------------
// Argument rendering
// ---------------------------------------------------------------------------

fn resolve_field<'a>(input: &'a serde_json::Value, field_path: &str) -> &'a serde_json::Value {
    let mut current = input;
    for part in field_path.split('.') {
        current = current.get(part).unwrap_or(&serde_json::Value::Null);
    }
    current
}

fn render_rust_arg(
    name: &str,
    value: &serde_json::Value,
    arg_type: &str,
    optional: bool,
    module: &str,
    fixture_id: &str,
) -> (Vec<String>, String) {
    if arg_type == "mock_url" {
        let lines = vec![format!(
            "let {name} = format!(\"{{}}/fixtures/{{}}\", std::env::var(\"MOCK_SERVER_URL\").expect(\"MOCK_SERVER_URL not set\"), \"{fixture_id}\");"
        )];
        return (lines, format!("&{name}"));
    }
    if arg_type == "handle" {
        // Generate a create_engine (or equivalent) call and pass the config.
        // If the fixture has input.config, serialize it as a json_object and pass it;
        // otherwise pass None.
        use heck::ToSnakeCase;
        let constructor_name = format!("create_{}", name.to_snake_case());
        let mut lines = Vec::new();
        if value.is_null() || value.is_object() && value.as_object().unwrap().is_empty() {
            lines.push(format!(
                "let {name} = {constructor_name}(None).expect(\"handle creation should succeed\");"
            ));
        } else {
            // Serialize the config JSON and deserialize at runtime.
            let json_literal = serde_json::to_string(value).unwrap_or_default();
            let escaped = json_literal.replace('\\', "\\\\").replace('"', "\\\"");
            lines.push(format!(
                "let {name}_config: CrawlConfig = serde_json::from_str(\"{escaped}\").expect(\"config should parse\");"
            ));
            lines.push(format!(
                "let {name} = {constructor_name}(Some({name}_config)).expect(\"handle creation should succeed\");"
            ));
        }
        return (lines, format!("&{name}"));
    }
    if arg_type == "json_object" {
        return render_json_object_arg(name, value, optional, module);
    }
    if value.is_null() && !optional {
        // Required arg with no fixture value: use a language-appropriate default.
        let default_val = match arg_type {
            "string" => "String::new()".to_string(),
            "int" | "integer" => "0".to_string(),
            "float" | "number" => "0.0_f64".to_string(),
            "bool" | "boolean" => "false".to_string(),
            _ => "Default::default()".to_string(),
        };
        // String args are passed by reference in Rust.
        let expr = if arg_type == "string" {
            format!("&{name}")
        } else {
            name.to_string()
        };
        return (vec![format!("let {name} = {default_val};")], expr);
    }
    let literal = json_to_rust_literal(value, arg_type);
    // String args are passed by reference in Rust.
    let pass_by_ref = arg_type == "string";
    let expr = |n: &str| if pass_by_ref { format!("&{n}") } else { n.to_string() };
    if optional && value.is_null() {
        (vec![format!("let {name} = None;")], expr(name))
    } else if optional {
        (vec![format!("let {name} = Some({literal});")], expr(name))
    } else {
        (vec![format!("let {name} = {literal};")], expr(name))
    }
}

/// Render a `json_object` argument: serialize the fixture JSON as a `serde_json::json!` literal
/// and deserialize it through serde at runtime. Type inference from the function signature
/// determines the concrete type, keeping the generator generic.
fn render_json_object_arg(
    name: &str,
    value: &serde_json::Value,
    optional: bool,
    _module: &str,
) -> (Vec<String>, String) {
    if value.is_null() && optional {
        return (vec![format!("let {name} = None;")], name.to_string());
    }

    // Fixture keys are camelCase; the Rust ConversionOptions type uses snake_case serde.
    // Normalize keys before building the json! literal so deserialization succeeds.
    let normalized = super::normalize_json_keys_to_snake_case(value);
    // Build the json! macro invocation from the fixture object.
    let json_literal = json_value_to_macro_literal(&normalized);
    let mut lines = Vec::new();
    lines.push(format!("let {name}_json = serde_json::json!({json_literal});"));
    // Deserialize to a concrete type inferred from the function signature.
    let deser_expr = format!("serde_json::from_value({name}_json).unwrap()");
    if optional {
        lines.push(format!("let {name} = Some({deser_expr});"));
    } else {
        lines.push(format!("let {name} = {deser_expr};"));
    }
    (lines, name.to_string())
}

/// Convert a `serde_json::Value` into a string suitable for the `serde_json::json!()` macro.
fn json_value_to_macro_literal(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => format!("{b}"),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => {
            let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{escaped}\"")
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_value_to_macro_literal).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(obj) => {
            let entries: Vec<String> = obj
                .iter()
                .map(|(k, v)| {
                    let escaped_key = k.replace('\\', "\\\\").replace('"', "\\\"");
                    format!("\"{escaped_key}\": {}", json_value_to_macro_literal(v))
                })
                .collect();
            format!("{{{}}}", entries.join(", "))
        }
    }
}

fn json_to_rust_literal(value: &serde_json::Value, arg_type: &str) -> String {
    match value {
        serde_json::Value::Null => "None".to_string(),
        serde_json::Value::Bool(b) => format!("{b}"),
        serde_json::Value::Number(n) => {
            if arg_type.contains("float") || arg_type.contains("f64") || arg_type.contains("f32") {
                if let Some(f) = n.as_f64() {
                    return format!("{f}_f64");
                }
            }
            n.to_string()
        }
        serde_json::Value::String(s) => rust_raw_string(s),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            let literal = rust_raw_string(&json_str);
            format!("serde_json::from_str({literal}).unwrap()")
        }
    }
}

// ---------------------------------------------------------------------------
// Assertion rendering
// ---------------------------------------------------------------------------

fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    _dep_name: &str,
    is_error_context: bool,
    unwrapped_fields: &[(String, String)], // (fixture_field, local_var)
    field_resolver: &FieldResolver,
) {
    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "    // skipped: field '{f}' not available on result type");
            return;
        }
    }

    // Determine field access expression:
    // 1. If the field was unwrapped to a local var, use that local var name.
    // 2. Otherwise, use the field resolver to generate the accessor.
    let field_access = match &assertion.field {
        Some(f) if !f.is_empty() => {
            if let Some((_, local_var)) = unwrapped_fields.iter().find(|(ff, _)| ff == f) {
                local_var.clone()
            } else {
                field_resolver.accessor(f, "rust", result_var)
            }
        }
        _ => result_var.to_string(),
    };

    // Check if this field was unwrapped (i.e., it is optional and was bound to a local).
    let is_unwrapped = assertion
        .field
        .as_ref()
        .is_some_and(|f| unwrapped_fields.iter().any(|(ff, _)| ff == f));

    match assertion.assertion_type.as_str() {
        "error" => {
            let _ = writeln!(out, "    assert!({result_var}.is_err(), \"expected call to fail\");");
            if let Some(serde_json::Value::String(msg)) = &assertion.value {
                let escaped = escape_rust(msg);
                let _ = writeln!(
                    out,
                    "    assert!({result_var}.as_ref().unwrap_err().to_string().contains(\"{escaped}\"), \"error message mismatch\");"
                );
            }
        }
        "not_error" => {
            // Handled at call site; nothing extra needed here.
        }
        "equals" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_rust_string(val);
                if is_error_context {
                    return;
                }
                // For string equality, trim trailing whitespace to handle trailing newlines
                // from the converter.
                if val.is_string() {
                    let _ = writeln!(
                        out,
                        "    assert_eq!({field_access}.trim(), {expected}, \"equals assertion failed\");"
                    );
                } else if val.is_boolean() {
                    // Use assert!/assert!(!...) for booleans — clippy prefers this over assert_eq!(_, true/false).
                    if val.as_bool() == Some(true) {
                        let _ = writeln!(out, "    assert!({field_access}, \"equals assertion failed\");");
                    } else {
                        let _ = writeln!(out, "    assert!(!{field_access}, \"equals assertion failed\");");
                    }
                } else {
                    // Wrap expected value in Some() for optional fields.
                    let is_opt = assertion.field.as_ref().is_some_and(|f| {
                        let resolved = field_resolver.resolve(f);
                        field_resolver.is_optional(resolved)
                    });
                    if is_opt
                        && !unwrapped_fields
                            .iter()
                            .any(|(ff, _)| assertion.field.as_ref() == Some(ff))
                    {
                        let _ = writeln!(
                            out,
                            "    assert_eq!({field_access}, Some({expected}), \"equals assertion failed\");"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "    assert_eq!({field_access}, {expected}, \"equals assertion failed\");"
                        );
                    }
                }
            }
        }
        "contains" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_rust_string(val);
                let line = format!(
                    "    assert!(format!(\"{{:?}}\", {field_access}).contains({expected}), \"expected to contain: {{}}\", {expected});"
                );
                let _ = writeln!(out, "{line}");
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let expected = value_to_rust_string(val);
                    let line = format!(
                        "    assert!(format!(\"{{:?}}\", {field_access}).contains({expected}), \"expected to contain: {{}}\", {expected});"
                    );
                    let _ = writeln!(out, "{line}");
                }
            }
        }
        "not_contains" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_rust_string(val);
                let line = format!(
                    "    assert!(!format!(\"{{:?}}\", {field_access}).contains({expected}), \"expected NOT to contain: {{}}\", {expected});"
                );
                let _ = writeln!(out, "{line}");
            }
        }
        "not_empty" => {
            if let Some(f) = &assertion.field {
                let resolved = field_resolver.resolve(f);
                if !is_unwrapped && field_resolver.is_optional(resolved) {
                    // Non-string optional field (e.g., Option<Struct>): use is_some()
                    let accessor = field_resolver.accessor(f, "rust", result_var);
                    let _ = writeln!(
                        out,
                        "    assert!({accessor}.is_some(), \"expected {f} to be present\");"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "    assert!(!{field_access}.is_empty(), \"expected non-empty value\");"
                    );
                }
            } else {
                let _ = writeln!(
                    out,
                    "    assert!(!{field_access}.is_empty(), \"expected non-empty value\");"
                );
            }
        }
        "is_empty" => {
            if let Some(f) = &assertion.field {
                let resolved = field_resolver.resolve(f);
                if !is_unwrapped && field_resolver.is_optional(resolved) {
                    let accessor = field_resolver.accessor(f, "rust", result_var);
                    let _ = writeln!(out, "    assert!({accessor}.is_none(), \"expected {f} to be absent\");");
                } else {
                    let _ = writeln!(out, "    assert!({field_access}.is_empty(), \"expected empty value\");");
                }
            } else {
                let _ = writeln!(out, "    assert!({field_access}.is_empty(), \"expected empty value\");");
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let checks: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let expected = value_to_rust_string(v);
                        format!("{field_access}.contains({expected})")
                    })
                    .collect();
                let joined = checks.join(" || ");
                let _ = writeln!(
                    out,
                    "    assert!({joined}, \"expected to contain at least one of the specified values\");"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                // Skip comparisons with negative values against unsigned types (.len() etc.)
                if val.as_f64().is_some_and(|n| n < 0.0) {
                    let _ = writeln!(
                        out,
                        "    // skipped: greater_than with negative value is always true for unsigned types"
                    );
                } else if val.as_u64() == Some(0) {
                    // Clippy prefers !is_empty() over len() > 0
                    let base = field_access.strip_suffix(".len()").unwrap_or(&field_access);
                    let _ = writeln!(out, "    assert!(!{base}.is_empty(), \"expected > 0\");");
                } else {
                    let lit = numeric_literal(val);
                    let _ = writeln!(out, "    assert!({field_access} > {lit}, \"expected > {lit}\");");
                }
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let lit = numeric_literal(val);
                let _ = writeln!(out, "    assert!({field_access} < {lit}, \"expected < {lit}\");");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                if val.as_u64() == Some(1) {
                    // Clippy prefers !is_empty() over len() >= 1
                    let base = field_access.strip_suffix(".len()").unwrap_or(&field_access);
                    let _ = writeln!(out, "    assert!(!{base}.is_empty(), \"expected >= 1\");");
                } else {
                    let lit = numeric_literal(val);
                    let _ = writeln!(out, "    assert!({field_access} >= {lit}, \"expected >= {lit}\");");
                }
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let lit = numeric_literal(val);
                let _ = writeln!(out, "    assert!({field_access} <= {lit}, \"expected <= {lit}\");");
            }
        }
        "starts_with" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_rust_string(val);
                let _ = writeln!(
                    out,
                    "    assert!({field_access}.starts_with({expected}), \"expected to start with: {{}}\", {expected});"
                );
            }
        }
        "ends_with" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_rust_string(val);
                let _ = writeln!(
                    out,
                    "    assert!({field_access}.ends_with({expected}), \"expected to end with: {{}}\", {expected});"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "    assert!({field_access}.len() >= {n}, \"expected length >= {n}, got {{}}\", {field_access}.len());"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "    assert!({field_access}.len() <= {n}, \"expected length <= {n}, got {{}}\", {field_access}.len());"
                    );
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if n <= 1 {
                        // Clippy prefers !is_empty() over len() >= 1
                        let base = field_access.strip_suffix(".len()").unwrap_or(&field_access);
                        let _ = writeln!(out, "    assert!(!{base}.is_empty(), \"expected >= {n}\");");
                    } else {
                        let _ = writeln!(
                            out,
                            "    assert!({field_access}.len() >= {n}, \"expected at least {n} elements, got {{}}\", {field_access}.len());"
                        );
                    }
                }
            }
        }
        other => {
            let _ = writeln!(out, "    // TODO: unsupported assertion type: {other}");
        }
    }
}

/// Convert a JSON numeric value to a Rust literal suitable for comparisons.
///
/// Whole numbers (no fractional part) are emitted as bare integer literals so
/// they are compatible with `usize`, `u64`, etc. (e.g., `.len()` results).
/// Numbers with a fractional component get the `_f64` suffix.
fn numeric_literal(value: &serde_json::Value) -> String {
    if let Some(n) = value.as_f64() {
        if n.fract() == 0.0 {
            // Whole number — emit without a type suffix so Rust can infer the
            // correct integer type from context (usize, u64, i64, …).
            return format!("{}", n as i64);
        }
        return format!("{n}_f64");
    }
    // Fallback: use the raw JSON representation.
    value.to_string()
}

fn value_to_rust_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => rust_raw_string(s),
        serde_json::Value::Bool(b) => format!("{b}"),
        serde_json::Value::Number(n) => n.to_string(),
        other => {
            let s = other.to_string();
            format!("\"{s}\"")
        }
    }
}
