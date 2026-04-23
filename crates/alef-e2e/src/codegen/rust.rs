//! Rust e2e test code generator.
//!
//! Generates `e2e/rust/Cargo.toml` and `tests/{category}_test.rs` files from
//! JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

use crate::config::E2eConfig;
use crate::escape::{escape_rust, rust_raw_string, sanitize_filename, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, CallbackAction, Fixture, FixtureGroup};
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
        // Check if any call config (default or named) uses json_object/handle args (needs serde_json dep).
        let all_call_configs = std::iter::once(&e2e_config.call).chain(e2e_config.calls.values());
        let needs_serde_json = all_call_configs
            .flat_map(|c| c.args.iter())
            .any(|a| a.arg_type == "json_object" || a.arg_type == "handle");

        // Check if any fixture in any group requires a mock HTTP server.
        let needs_mock_server = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| !is_skipped(f, "rust") && f.needs_mock_server());

        // Tokio is needed when any test is async (mock server or async call config).
        let any_async_call = std::iter::once(&e2e_config.call)
            .chain(e2e_config.calls.values())
            .any(|c| c.r#async);
        let needs_tokio = needs_mock_server || any_async_call;

        let crate_version = resolve_crate_version(e2e_config);
        files.push(GeneratedFile {
            path: output_base.join("Cargo.toml"),
            content: render_cargo_toml(
                &crate_name,
                &dep_name,
                &crate_path,
                needs_serde_json,
                needs_mock_server,
                needs_tokio,
                e2e_config.dep_mode,
                crate_version.as_deref(),
                &alef_config.crate_config.features,
            ),
            generated_header: true,
        });

        // Generate mock_server.rs when at least one fixture uses mock_response.
        if needs_mock_server {
            files.push(GeneratedFile {
                path: output_base.join("tests").join("mock_server.rs"),
                content: render_mock_server_module(),
                generated_header: true,
            });
            // Generate standalone mock-server binary for cross-language e2e suites.
            files.push(GeneratedFile {
                path: output_base.join("src").join("main.rs"),
                content: render_mock_server_binary(),
                generated_header: true,
            });
        }

        // Per-category test files.
        for group in groups {
            let fixtures: Vec<&Fixture> = group.fixtures.iter().filter(|f| !is_skipped(f, "rust")).collect();

            if fixtures.is_empty() {
                continue;
            }

            let filename = format!("{}_test.rs", sanitize_filename(&group.category));
            let content = render_test_file(&group.category, &fixtures, e2e_config, &dep_name, needs_mock_server);

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

fn resolve_function_name_for_call(call_config: &crate::config::CallConfig) -> String {
    call_config
        .overrides
        .get("rust")
        .and_then(|o| o.function.clone())
        .unwrap_or_else(|| call_config.function.clone())
}

fn resolve_module(e2e_config: &E2eConfig, dep_name: &str) -> String {
    resolve_module_for_call(&e2e_config.call, dep_name)
}

fn resolve_module_for_call(call_config: &crate::config::CallConfig, dep_name: &str) -> String {
    // For Rust, the module name is the crate identifier (underscores).
    // Priority: override.crate_name > override.module > dep_name
    let overrides = call_config.overrides.get("rust");
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

#[allow(clippy::too_many_arguments)]
fn render_cargo_toml(
    crate_name: &str,
    dep_name: &str,
    crate_path: &str,
    needs_serde_json: bool,
    needs_mock_server: bool,
    needs_tokio: bool,
    dep_mode: crate::config::DependencyMode,
    version: Option<&str>,
    features: &[String],
) -> String {
    let e2e_name = format!("{dep_name}-e2e-rust");
    let mut effective_features: Vec<&str> = features.iter().map(|s| s.as_str()).collect();
    if needs_serde_json && !effective_features.contains(&"serde") {
        effective_features.push("serde");
    }
    let features_str = if effective_features.is_empty() {
        String::new()
    } else {
        format!(", default-features = false, features = {:?}", effective_features)
    };
    let dep_spec = match dep_mode {
        crate::config::DependencyMode::Registry => {
            let ver = version.unwrap_or("0.1.0");
            if crate_name != dep_name {
                format!("{dep_name} = {{ package = \"{crate_name}\", version = \"{ver}\"{features_str} }}")
            } else if effective_features.is_empty() {
                format!("{dep_name} = \"{ver}\"")
            } else {
                format!("{dep_name} = {{ version = \"{ver}\"{features_str} }}")
            }
        }
        crate::config::DependencyMode::Local => {
            if crate_name != dep_name {
                format!("{dep_name} = {{ package = \"{crate_name}\", path = \"{crate_path}\"{features_str} }}")
            } else if effective_features.is_empty() {
                format!("{dep_name} = {{ path = \"{crate_path}\" }}")
            } else {
                format!("{dep_name} = {{ path = \"{crate_path}\"{features_str} }}")
            }
        }
    };
    let serde_line = if needs_serde_json { "\nserde_json = \"1\"" } else { "" };
    // In registry mode the generated Cargo.toml is a standalone project, so add
    // an empty [workspace] table to opt out of parent workspace discovery.
    // In local mode the e2e crate is typically listed as a workspace member of
    // the parent project, so adding [workspace] would create a conflicting
    // second workspace root — omit it.
    // Always add [workspace] — both registry and local e2e crates are standalone
    // projects that must not inherit the parent workspace.
    let workspace_section = "\n[workspace]\n";
    // Mock server requires axum (HTTP router) and tokio-stream (SSE streaming).
    // The standalone binary additionally needs serde (derive) and walkdir.
    let mock_lines = if needs_mock_server {
        "\naxum = \"0.8\"\ntokio-stream = \"0.1\"\nserde = { version = \"1\", features = [\"derive\"] }\nwalkdir = \"2\""
    } else {
        ""
    };
    let mut machete_ignored: Vec<&str> = Vec::new();
    if needs_serde_json {
        machete_ignored.push("\"serde_json\"");
    }
    if needs_mock_server {
        machete_ignored.push("\"axum\"");
        machete_ignored.push("\"tokio-stream\"");
        machete_ignored.push("\"serde\"");
        machete_ignored.push("\"walkdir\"");
    }
    let machete_section = if machete_ignored.is_empty() {
        String::new()
    } else {
        format!(
            "\n[package.metadata.cargo-machete]\nignored = [{}]\n",
            machete_ignored.join(", ")
        )
    };
    let tokio_line = if needs_tokio {
        "\ntokio = { version = \"1\", features = [\"full\"] }"
    } else {
        ""
    };
    let bin_section = if needs_mock_server {
        "\n[[bin]]\nname = \"mock-server\"\npath = \"src/main.rs\"\n"
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
{bin_section}
[dependencies]
{dep_spec}{serde_line}{mock_lines}{tokio_line}
{machete_section}"#
    )
}

fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    dep_name: &str,
    needs_mock_server: bool,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// This file is auto-generated by alef. DO NOT EDIT.");
    let _ = writeln!(out, "//! E2e tests for category: {category}");
    let _ = writeln!(out);

    let module = resolve_module(e2e_config, dep_name);
    let field_resolver = FieldResolver::new(
        &e2e_config.fields,
        &e2e_config.fields_optional,
        &e2e_config.result_fields,
        &e2e_config.fields_array,
    );

    // Collect all unique (module, function) pairs needed across all fixtures in this file.
    // Fixtures that name a specific call may use a different function (and module) than
    // the default [e2e.call] config.
    let mut imported: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();
    for fixture in fixtures.iter() {
        let call_config = e2e_config.resolve_call(fixture.call.as_deref());
        let fn_name = resolve_function_name_for_call(call_config);
        let mod_name = resolve_module_for_call(call_config, dep_name);
        imported.insert((mod_name, fn_name));
    }
    // Emit use statements, grouping by module when possible.
    let mut by_module: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for (mod_name, fn_name) in &imported {
        by_module.entry(mod_name.clone()).or_default().push(fn_name.clone());
    }
    for (mod_name, fns) in &by_module {
        if fns.len() == 1 {
            let _ = writeln!(out, "use {mod_name}::{};", fns[0]);
        } else {
            let joined = fns.join(", ");
            let _ = writeln!(out, "use {mod_name}::{{{joined}}};");
        }
    }

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

    // Import mock_server module when any fixture in this file uses mock_response.
    let file_needs_mock = needs_mock_server && fixtures.iter().any(|f| f.needs_mock_server());
    if file_needs_mock {
        let _ = writeln!(out, "mod mock_server;");
        let _ = writeln!(out, "use mock_server::{{MockRoute, MockServer}};");
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
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let function_name = resolve_function_name_for_call(call_config);
    let module = resolve_module_for_call(call_config, dep_name);
    let result_var = &call_config.result_var;
    let has_mock = fixture.needs_mock_server();

    // Tests with a mock server are always async (Axum requires a Tokio runtime).
    let is_async = call_config.r#async || has_mock;
    if is_async {
        let _ = writeln!(out, "#[tokio::test]");
        let _ = writeln!(out, "async fn test_{fn_name}() {{");
    } else {
        let _ = writeln!(out, "#[test]");
        let _ = writeln!(out, "fn test_{fn_name}() {{");
    }
    let _ = writeln!(out, "    // {description}");

    // Emit mock server setup before building arguments so arg expressions can
    // reference `mock_server.url` when needed.
    if has_mock {
        render_mock_server_setup(out, fixture, e2e_config);
    }

    // Check if any assertion is an error assertion.
    let has_error_assertion = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Emit input variable bindings from args config.
    let mut arg_exprs: Vec<String> = Vec::new();
    for arg in &call_config.args {
        let value = resolve_field(&fixture.input, &arg.field);
        let var_name = &arg.name;
        let (bindings, expr) = render_rust_arg(
            var_name,
            value,
            &arg.arg_type,
            arg.optional,
            &module,
            &fixture.id,
            if has_mock {
                Some("mock_server.url.as_str()")
            } else {
                None
            },
        );
        for binding in &bindings {
            let _ = writeln!(out, "    {binding}");
        }
        arg_exprs.push(expr);
    }

    // Emit visitor if present in fixture.
    if let Some(visitor_spec) = &fixture.visitor {
        let _ = writeln!(out, "    struct _TestVisitor;");
        let _ = writeln!(out, "    impl {} for _TestVisitor {{", resolve_visitor_trait(&module));
        for (method_name, action) in &visitor_spec.callbacks {
            emit_rust_visitor_method(out, method_name, action);
        }
        let _ = writeln!(out, "    }}");
        let _ = writeln!(
            out,
            "    let visitor = std::rc::Rc::new(std::cell::RefCell::new(_TestVisitor));"
        );
        arg_exprs.push("Some(visitor)".to_string());
    }

    let args_str = arg_exprs.join(", ");

    let await_suffix = if is_async { ".await" } else { "" };

    let result_is_tree = call_config.result_var == "tree";

    if has_error_assertion {
        let _ = writeln!(out, "    let {result_var} = {function_name}({args_str}){await_suffix};");
        // Render error assertions.
        for assertion in &fixture.assertions {
            render_assertion(
                out,
                assertion,
                result_var,
                &module,
                dep_name,
                true,
                &[],
                field_resolver,
                result_is_tree,
            );
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
        if a.assertion_type == "method_result" {
            // method_result assertions that would generate only a TODO comment don't use the
            // result variable. These are: missing `method` field, or unsupported `check` type.
            let supported_checks = [
                "equals",
                "is_true",
                "is_false",
                "greater_than_or_equal",
                "count_min",
                "is_error",
                "contains",
                "not_empty",
                "is_empty",
            ];
            let check = a.check.as_deref().unwrap_or("is_true");
            if a.method.is_none() || !supported_checks.contains(&check) {
                return false;
            }
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

    // Detect Option-returning functions: only skip unwrap when ALL assertions are
    // pure emptiness/bool checks with NO field access (is_none/is_some on the result itself).
    // If any assertion accesses a field (e.g. `html`), we need the inner value, so unwrap.
    let has_field_access = fixture
        .assertions
        .iter()
        .any(|a| a.field.as_ref().is_some_and(|f| !f.is_empty()));
    let only_emptiness_checks = !has_not_error
        && !has_field_access
        && fixture.assertions.iter().all(|a| {
            matches!(
                a.assertion_type.as_str(),
                "is_empty" | "is_false" | "not_empty" | "is_true"
            )
        });

    if only_emptiness_checks {
        // Option-returning: don't unwrap, emit is_none/is_some checks directly
        let _ = writeln!(
            out,
            "    let {result_binding} = {function_name}({args_str}){await_suffix};"
        );
    } else if has_not_error || !fixture.assertions.is_empty() {
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
            &module,
            dep_name,
            false,
            &unwrapped_fields,
            field_resolver,
            result_is_tree,
        );
    }

    let _ = writeln!(out, "}}");
}

// ---------------------------------------------------------------------------
// Argument rendering
// ---------------------------------------------------------------------------

fn resolve_field<'a>(input: &'a serde_json::Value, field_path: &str) -> &'a serde_json::Value {
    // Field paths in call config are "input.path", "input.config", etc.
    // Since we already receive `fixture.input`, strip the leading "input." prefix.
    let path = field_path.strip_prefix("input.").unwrap_or(field_path);
    let mut current = input;
    for part in path.split('.') {
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
    mock_base_url: Option<&str>,
) -> (Vec<String>, String) {
    if arg_type == "mock_url" {
        let lines = vec![format!(
            "let {name} = format!(\"{{}}/fixtures/{{}}\", std::env::var(\"MOCK_SERVER_URL\").expect(\"MOCK_SERVER_URL not set\"), \"{fixture_id}\");"
        )];
        return (lines, format!("&{name}"));
    }
    // When the arg is a base_url and a mock server is running, use the mock server URL.
    if arg_type == "base_url" {
        if let Some(url_expr) = mock_base_url {
            return (vec![], url_expr.to_string());
        }
        // No mock server: fall through to string handling below.
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
    // Bytes args are strings passed as .as_bytes().
    let pass_by_ref = arg_type == "string" || arg_type == "bytes";
    let expr = |n: &str| {
        if arg_type == "bytes" {
            format!("{n}.as_bytes()")
        } else if pass_by_ref {
            format!("&{n}")
        } else {
            n.to_string()
        }
    };
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
        // Use Default::default() and pass by reference — Rust functions typically
        // take &T not Option<T> for config params.
        return (vec![format!("let {name} = Default::default();")], format!("&{name}"));
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
        (lines, format!("&{name}"))
    } else {
        lines.push(format!("let {name} = {deser_expr};"));
        (lines, format!("&{name}"))
    }
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
// Mock server helpers
// ---------------------------------------------------------------------------

/// Emit mock server setup lines into a test function body.
///
/// Builds `MockRoute` objects from the fixture's `mock_response` and starts
/// the server.  The resulting `mock_server` variable is in scope for the rest
/// of the test function.
fn render_mock_server_setup(out: &mut String, fixture: &Fixture, e2e_config: &E2eConfig) {
    let mock = match fixture.mock_response.as_ref() {
        Some(m) => m,
        None => return,
    };

    // Resolve the HTTP path and method from the call config.
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let path = call_config.path.as_deref().unwrap_or("/");
    let method = call_config.method.as_deref().unwrap_or("POST");

    let status = mock.status;

    if let Some(chunks) = &mock.stream_chunks {
        // Streaming SSE response.
        let _ = writeln!(out, "    let mock_route = MockRoute {{");
        let _ = writeln!(out, "        path: \"{path}\",");
        let _ = writeln!(out, "        method: \"{method}\",");
        let _ = writeln!(out, "        status: {status},");
        let _ = writeln!(out, "        body: String::new(),");
        let _ = writeln!(out, "        stream_chunks: vec![");
        for chunk in chunks {
            let chunk_str = match chunk {
                serde_json::Value::String(s) => rust_raw_string(s),
                other => {
                    let s = serde_json::to_string(other).unwrap_or_default();
                    rust_raw_string(&s)
                }
            };
            let _ = writeln!(out, "            {chunk_str}.to_string(),");
        }
        let _ = writeln!(out, "        ],");
        let _ = writeln!(out, "    }};");
    } else {
        // Non-streaming JSON response.
        let body_str = match &mock.body {
            Some(b) => {
                let s = serde_json::to_string(b).unwrap_or_default();
                rust_raw_string(&s)
            }
            None => rust_raw_string("{}"),
        };
        let _ = writeln!(out, "    let mock_route = MockRoute {{");
        let _ = writeln!(out, "        path: \"{path}\",");
        let _ = writeln!(out, "        method: \"{method}\",");
        let _ = writeln!(out, "        status: {status},");
        let _ = writeln!(out, "        body: {body_str}.to_string(),");
        let _ = writeln!(out, "        stream_chunks: vec![],");
        let _ = writeln!(out, "    }};");
    }

    let _ = writeln!(out, "    let mock_server = MockServer::start(vec![mock_route]).await;");
}

/// Generate the complete `mock_server.rs` module source.
fn render_mock_server_module() -> String {
    // This is parameterized Axum mock server code identical in structure to
    // liter-llm's mock_server.rs but without any project-specific imports.
    r#"// This file is auto-generated by alef. DO NOT EDIT.
//
// Minimal axum-based mock HTTP server for e2e tests.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::net::TcpListener;

/// A single mock route: match by path + method, return a configured response.
#[derive(Clone, Debug)]
pub struct MockRoute {
    /// URL path to match, e.g. `"/v1/chat/completions"`.
    pub path: &'static str,
    /// HTTP method to match, e.g. `"POST"` or `"GET"`.
    pub method: &'static str,
    /// HTTP status code to return.
    pub status: u16,
    /// Response body JSON string (used when `stream_chunks` is empty).
    pub body: String,
    /// Ordered SSE data payloads for streaming responses.
    /// Each entry becomes `data: <chunk>\n\n` in the response.
    /// A final `data: [DONE]\n\n` is always appended.
    pub stream_chunks: Vec<String>,
}

struct ServerState {
    routes: Vec<MockRoute>,
}

pub struct MockServer {
    /// Base URL of the mock server, e.g. `"http://127.0.0.1:54321"`.
    pub url: String,
    handle: tokio::task::JoinHandle<()>,
}

impl MockServer {
    /// Start a mock server with the given routes.  Binds to a random port on
    /// localhost and returns immediately once the server is listening.
    pub async fn start(routes: Vec<MockRoute>) -> Self {
        let state = Arc::new(ServerState { routes });

        let app = Router::new().fallback(handle_request).with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind mock server port");
        let addr: SocketAddr = listener.local_addr().expect("Failed to get local addr");
        let url = format!("http://{addr}");

        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("Mock server failed");
        });

        MockServer { url, handle }
    }

    /// Stop the mock server.
    pub fn shutdown(self) {
        self.handle.abort();
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn handle_request(State(state): State<Arc<ServerState>>, req: Request<Body>) -> Response {
    let path = req.uri().path().to_owned();
    let method = req.method().as_str().to_uppercase();

    for route in &state.routes {
        if route.path == path && route.method.to_uppercase() == method {
            let status =
                StatusCode::from_u16(route.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

            if !route.stream_chunks.is_empty() {
                // Build SSE body: data: <chunk>\n\n ... data: [DONE]\n\n
                let mut sse = String::new();
                for chunk in &route.stream_chunks {
                    sse.push_str("data: ");
                    sse.push_str(chunk);
                    sse.push_str("\n\n");
                }
                sse.push_str("data: [DONE]\n\n");

                return Response::builder()
                    .status(status)
                    .header("content-type", "text/event-stream")
                    .header("cache-control", "no-cache")
                    .body(Body::from(sse))
                    .unwrap()
                    .into_response();
            }

            return Response::builder()
                .status(status)
                .header("content-type", "application/json")
                .body(Body::from(route.body.clone()))
                .unwrap()
                .into_response();
        }
    }

    // No matching route → 404.
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from(format!("No mock route for {method} {path}")))
        .unwrap()
        .into_response()
}
"#
    .to_string()
}

/// Generate the `src/main.rs` for the standalone mock server binary.
///
/// The binary:
/// - Reads all `*.json` fixture files from a fixtures directory (default `../../fixtures`).
/// - For each fixture that has a `mock_response` field, registers a route at
///   `/fixtures/{fixture_id}` returning the configured status/body/SSE chunks.
/// - Binds to `127.0.0.1:0` (random port), prints `MOCK_SERVER_URL=http://...`
///   to stdout, then waits until stdin is closed for clean teardown.
///
/// This binary is intended for cross-language e2e suites (WASM, Node) that
/// spawn it as a child process and read the URL from its stdout.
fn render_mock_server_binary() -> String {
    r#"// This file is auto-generated by alef. DO NOT EDIT.
//
// Standalone mock HTTP server binary for cross-language e2e tests.
// Reads fixture JSON files and serves mock responses on /fixtures/{fixture_id}.
//
// Usage: mock-server [fixtures-dir]
//   fixtures-dir defaults to "../../fixtures"
//
// Prints `MOCK_SERVER_URL=http://127.0.0.1:<port>` to stdout once listening,
// then blocks until stdin is closed (parent process exit triggers cleanup).

use std::collections::HashMap;
use std::io::{self, BufRead};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Fixture types (mirrors alef-e2e's fixture.rs for runtime deserialization)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct MockResponse {
    status: u16,
    #[serde(default)]
    body: Option<serde_json::Value>,
    #[serde(default)]
    stream_chunks: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    id: String,
    #[serde(default)]
    mock_response: Option<MockResponse>,
}

// ---------------------------------------------------------------------------
// Route table
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct MockRoute {
    status: u16,
    body: String,
    stream_chunks: Vec<String>,
}

type RouteTable = Arc<HashMap<String, MockRoute>>;

// ---------------------------------------------------------------------------
// Axum handler
// ---------------------------------------------------------------------------

async fn handle_request(State(routes): State<RouteTable>, req: Request<Body>) -> Response {
    let path = req.uri().path().to_owned();

    // Try exact match first
    if let Some(route) = routes.get(&path) {
        return serve_route(route);
    }

    // Try prefix match: find a route that is a prefix of the request path
    // This allows /fixtures/basic_chat/v1/chat/completions to match /fixtures/basic_chat
    for (route_path, route) in routes.iter() {
        if path.starts_with(route_path) && (path.len() == route_path.len() || path.as_bytes()[route_path.len()] == b'/') {
            return serve_route(route);
        }
    }

    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from(format!("No mock route for {path}")))
        .unwrap()
        .into_response()
}

fn serve_route(route: &MockRoute) -> Response {
    let status = StatusCode::from_u16(route.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    if !route.stream_chunks.is_empty() {
        let mut sse = String::new();
        for chunk in &route.stream_chunks {
            sse.push_str("data: ");
            sse.push_str(chunk);
            sse.push_str("\n\n");
        }
        sse.push_str("data: [DONE]\n\n");

        return Response::builder()
            .status(status)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .body(Body::from(sse))
            .unwrap()
            .into_response();
    }

    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(route.body.clone()))
        .unwrap()
        .into_response()
}

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

fn load_routes(fixtures_dir: &Path) -> HashMap<String, MockRoute> {
    let mut routes = HashMap::new();
    load_routes_recursive(fixtures_dir, &mut routes);
    routes
}

fn load_routes_recursive(dir: &Path, routes: &mut HashMap<String, MockRoute>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("warning: cannot read directory {}: {err}", dir.display());
            return;
        }
    };

    let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            load_routes_recursive(&path, routes);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if filename == "schema.json" || filename.starts_with('_') {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(err) => {
                    eprintln!("warning: cannot read {}: {err}", path.display());
                    continue;
                }
            };
            let fixtures: Vec<Fixture> = if content.trim_start().starts_with('[') {
                match serde_json::from_str(&content) {
                    Ok(v) => v,
                    Err(err) => {
                        eprintln!("warning: cannot parse {}: {err}", path.display());
                        continue;
                    }
                }
            } else {
                match serde_json::from_str::<Fixture>(&content) {
                    Ok(f) => vec![f],
                    Err(err) => {
                        eprintln!("warning: cannot parse {}: {err}", path.display());
                        continue;
                    }
                }
            };

            for fixture in fixtures {
                if let Some(mock) = fixture.mock_response {
                    let route_path = format!("/fixtures/{}", fixture.id);
                    let body = mock
                        .body
                        .as_ref()
                        .map(|b| serde_json::to_string(b).unwrap_or_default())
                        .unwrap_or_default();
                    let stream_chunks = mock
                        .stream_chunks
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| match c {
                            serde_json::Value::String(s) => s,
                            other => serde_json::to_string(&other).unwrap_or_default(),
                        })
                        .collect();
                    routes.insert(route_path, MockRoute { status: mock.status, body, stream_chunks });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let fixtures_dir_arg = std::env::args().nth(1).unwrap_or_else(|| "../../fixtures".to_string());
    let fixtures_dir = Path::new(&fixtures_dir_arg);

    let routes = load_routes(fixtures_dir);
    eprintln!("mock-server: loaded {} routes from {}", routes.len(), fixtures_dir.display());

    let route_table: RouteTable = Arc::new(routes);
    let app = Router::new().fallback(handle_request).with_state(route_table);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock-server: failed to bind port");
    let addr: SocketAddr = listener.local_addr().expect("mock-server: failed to get local addr");

    // Print the URL so the parent process can read it.
    println!("MOCK_SERVER_URL=http://{addr}");
    // Flush stdout explicitly so the parent does not block waiting.
    use std::io::Write;
    std::io::stdout().flush().expect("mock-server: failed to flush stdout");

    // Spawn the server in the background.
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock-server: server error");
    });

    // Block until stdin is closed — the parent process controls lifetime.
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    while lines.next().is_some() {}
}
"#
    .to_string()
}

// ---------------------------------------------------------------------------
// Assertion rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    module: &str,
    _dep_name: &str,
    is_error_context: bool,
    unwrapped_fields: &[(String, String)], // (fixture_field, local_var)
    field_resolver: &FieldResolver,
    result_is_tree: bool,
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
    // 2. When the result is a Tree, map pseudo-field names to correct Rust expressions.
    // 3. Otherwise, use the field resolver to generate the accessor.
    let field_access = match &assertion.field {
        Some(f) if !f.is_empty() => {
            if let Some((_, local_var)) = unwrapped_fields.iter().find(|(ff, _)| ff == f) {
                local_var.clone()
            } else if result_is_tree {
                // Tree is an opaque type — its "fields" are accessed via root_node() or
                // free functions. Map known pseudo-field names to correct Rust expressions.
                tree_field_access_expr(f, result_var, module)
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
                // No field: assertion on the result itself. Use is_some() for Option types.
                let _ = writeln!(
                    out,
                    "    assert!({field_access}.is_some(), \"expected non-empty value\");"
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
                // No field: assertion on the result itself. Use is_none() for Option types.
                let _ = writeln!(out, "    assert!({field_access}.is_none(), \"expected empty value\");");
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
                let lit = numeric_literal(val);
                if val.as_u64() == Some(1) && field_access.ends_with(".len()") {
                    // Clippy prefers !is_empty() over len() >= 1 for collections.
                    // Only apply when the expression is already a `.len()` call so we
                    // don't mistakenly call `.is_empty()` on numeric (usize) fields.
                    let base = field_access.strip_suffix(".len()").unwrap_or(&field_access);
                    let _ = writeln!(out, "    assert!(!{base}.is_empty(), \"expected >= 1\");");
                } else {
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
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "    assert_eq!({field_access}.len(), {n}, \"expected exactly {n} elements, got {{}}\", {field_access}.len());"
                    );
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "    assert!({field_access}, \"expected true\");");
        }
        "is_false" => {
            let _ = writeln!(out, "    assert!(!{field_access}, \"expected false\");");
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                // Build the call expression. When the result is a tree-sitter Tree (an opaque
                // type), methods like `root_child_count` do not exist on `Tree` directly —
                // they are free functions in the crate or are accessed via `root_node()`.
                let call_expr = if result_is_tree {
                    build_tree_call_expr(field_access.as_str(), method_name, assertion.args.as_ref(), module)
                } else if let Some(args) = &assertion.args {
                    let arg_lit = json_to_rust_literal(args, "");
                    format!("{field_access}.{method_name}({arg_lit})")
                } else {
                    format!("{field_access}.{method_name}()")
                };

                // Determine whether the call expression returns a numeric type so we can
                // choose the right comparison strategy for `greater_than_or_equal`.
                let returns_numeric = result_is_tree && is_tree_numeric_method(method_name);

                let check = assertion.check.as_deref().unwrap_or("is_true");
                match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            if val.is_boolean() {
                                if val.as_bool() == Some(true) {
                                    let _ = writeln!(
                                        out,
                                        "    assert!({call_expr}, \"method_result equals assertion failed\");"
                                    );
                                } else {
                                    let _ = writeln!(
                                        out,
                                        "    assert!(!{call_expr}, \"method_result equals assertion failed\");"
                                    );
                                }
                            } else {
                                let expected = value_to_rust_string(val);
                                let _ = writeln!(
                                    out,
                                    "    assert_eq!({call_expr}, {expected}, \"method_result equals assertion failed\");"
                                );
                            }
                        }
                    }
                    "is_true" => {
                        let _ = writeln!(
                            out,
                            "    assert!({call_expr}, \"method_result is_true assertion failed\");"
                        );
                    }
                    "is_false" => {
                        let _ = writeln!(
                            out,
                            "    assert!(!{call_expr}, \"method_result is_false assertion failed\");"
                        );
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let lit = numeric_literal(val);
                            if returns_numeric {
                                // Numeric return (e.g., child_count()) — always use >= comparison.
                                let _ = writeln!(out, "    assert!({call_expr} >= {lit}, \"expected >= {lit}\");");
                            } else if val.as_u64() == Some(1) {
                                // Clippy prefers !is_empty() over len() >= 1 for collections.
                                let _ = writeln!(out, "    assert!(!{call_expr}.is_empty(), \"expected >= 1\");");
                            } else {
                                let _ = writeln!(out, "    assert!({call_expr} >= {lit}, \"expected >= {lit}\");");
                            }
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            if n <= 1 {
                                let _ = writeln!(out, "    assert!(!{call_expr}.is_empty(), \"expected >= {n}\");");
                            } else {
                                let _ = writeln!(
                                    out,
                                    "    assert!({call_expr}.len() >= {n}, \"expected at least {n} elements, got {{}}\", {call_expr}.len());"
                                );
                            }
                        }
                    }
                    "is_error" => {
                        // For is_error we need the raw Result without .unwrap().
                        let raw_call = call_expr.strip_suffix(".unwrap()").unwrap_or(&call_expr);
                        let _ = writeln!(
                            out,
                            "    assert!({raw_call}.is_err(), \"expected method to return error\");"
                        );
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            let expected = value_to_rust_string(val);
                            let _ = writeln!(
                                out,
                                "    assert!({call_expr}.contains({expected}), \"expected result to contain {{}}\", {expected});"
                            );
                        }
                    }
                    "not_empty" => {
                        let _ = writeln!(
                            out,
                            "    assert!(!{call_expr}.is_empty(), \"expected non-empty result\");"
                        );
                    }
                    "is_empty" => {
                        let _ = writeln!(out, "    assert!({call_expr}.is_empty(), \"expected empty result\");");
                    }
                    other_check => {
                        panic!("Rust e2e generator: unsupported method_result check type: {other_check}");
                    }
                }
            } else {
                panic!("Rust e2e generator: method_result assertion missing 'method' field");
            }
        }
        other => {
            panic!("Rust e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Translate a fixture pseudo-field name on a `tree_sitter::Tree` into the
/// correct Rust accessor expression.
///
/// When an assertion uses `field: "root_child_count"` on a tree result, the
/// field resolver would naively emit `tree.root_child_count` — which is invalid
/// because `Tree` is an opaque type with no such field.  This function maps the
/// pseudo-field to the correct Rust expression instead.
fn tree_field_access_expr(field: &str, result_var: &str, module: &str) -> String {
    match field {
        "root_child_count" => format!("{result_var}.root_node().child_count()"),
        "root_node_type" => format!("{result_var}.root_node().kind()"),
        "named_children_count" => format!("{result_var}.root_node().named_child_count()"),
        "has_error_nodes" => format!("{module}::tree_has_error_nodes(&{result_var})"),
        "error_count" | "tree_error_count" => format!("{module}::tree_error_count(&{result_var})"),
        "tree_to_sexp" => format!("{module}::tree_to_sexp(&{result_var})"),
        // Unknown pseudo-field: fall back to direct field access (will likely fail to compile,
        // but gives the developer a useful error pointing to the fixture).
        other => format!("{result_var}.{other}"),
    }
}

/// Build a Rust call expression for a logical "method" on a `tree_sitter::Tree`.
///
/// `Tree` is an opaque type — it does not expose methods like `root_child_count`.
/// Instead, these are either free functions in the crate or are accessed via
/// `tree.root_node().<method>()`. This function translates the fixture-level
/// method name into the correct Rust expression.
fn build_tree_call_expr(
    field_access: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
    module: &str,
) -> String {
    match method_name {
        "root_child_count" => format!("{field_access}.root_node().child_count()"),
        "root_node_type" => format!("{field_access}.root_node().kind()"),
        "named_children_count" => format!("{field_access}.root_node().named_child_count()"),
        "has_error_nodes" => format!("{module}::tree_has_error_nodes(&{field_access})"),
        "error_count" | "tree_error_count" => format!("{module}::tree_error_count(&{field_access})"),
        "tree_to_sexp" => format!("{module}::tree_to_sexp(&{field_access})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{module}::tree_contains_node_type(&{field_access}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{module}::find_nodes_by_type(&{field_access}, \"{node_type}\")")
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
            // Use a raw string for the query to avoid escaping issues.
            // run_query returns Result — unwrap it for assertion access.
            format!(
                "{module}::run_query(&{field_access}, \"{language}\", r#\"{query_source}\"#, source.as_bytes()).unwrap()"
            )
        }
        // Fallback: try as a plain method call.
        _ => {
            if let Some(args) = args {
                let arg_lit = json_to_rust_literal(args, "");
                format!("{field_access}.{method_name}({arg_lit})")
            } else {
                format!("{field_access}.{method_name}()")
            }
        }
    }
}

/// Returns `true` when the tree method name produces a numeric result (usize/u64),
/// meaning `>= N` comparisons should use direct numeric comparison rather than
/// `.is_empty()` (which only works for collections).
fn is_tree_numeric_method(method_name: &str) -> bool {
    matches!(
        method_name,
        "root_child_count" | "named_children_count" | "error_count" | "tree_error_count"
    )
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

// ---------------------------------------------------------------------------
// Visitor generation
// ---------------------------------------------------------------------------

/// Resolve the visitor trait name based on module.
fn resolve_visitor_trait(module: &str) -> String {
    // For html_to_markdown modules, use HtmlVisitor
    if module.contains("html_to_markdown") {
        "HtmlVisitor".to_string()
    } else {
        // Default fallback for other modules
        "Visitor".to_string()
    }
}

/// Emit a Rust visitor method for a callback action.
fn emit_rust_visitor_method(out: &mut String, method_name: &str, action: &CallbackAction) {
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

    let _ = writeln!(out, "        fn {method_name}(&self, {params}) -> VisitResult {{");
    match action {
        CallbackAction::Skip => {
            let _ = writeln!(out, "            VisitResult::Skip");
        }
        CallbackAction::Continue => {
            let _ = writeln!(out, "            VisitResult::Continue");
        }
        CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "            VisitResult::PreserveHtml");
        }
        CallbackAction::Custom { output } => {
            let escaped = escape_rust(output);
            let _ = writeln!(out, "            VisitResult::Custom({escaped}.to_string())");
        }
        CallbackAction::CustomTemplate { template } => {
            let _ = writeln!(out, "            VisitResult::Custom(format!(\"{template}\"))");
        }
    }
    let _ = writeln!(out, "        }}");
}
