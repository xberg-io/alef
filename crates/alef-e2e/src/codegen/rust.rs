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

        // Check if any fixture in any group requires a mock HTTP server.
        let needs_mock_server = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| !is_skipped(f, "rust") && f.needs_mock_server());

        let crate_version = resolve_crate_version(e2e_config);
        files.push(GeneratedFile {
            path: output_base.join("Cargo.toml"),
            content: render_cargo_toml(
                &crate_name,
                &dep_name,
                &crate_path,
                needs_serde_json,
                needs_mock_server,
                e2e_config.dep_mode,
                crate_version.as_deref(),
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
    needs_mock_server: bool,
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
    // Always add [workspace] — even in local mode the e2e crate lives outside
    // the parent workspace members list and needs its own workspace declaration.
    let workspace_section = "\n[workspace]\n";
    // Mock server requires axum (HTTP router) and tokio-stream (SSE streaming).
    let mock_lines = if needs_mock_server {
        "\naxum = \"0.8\"\ntokio-stream = \"0.1\""
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
    }
    let machete_section = if machete_ignored.is_empty() {
        String::new()
    } else {
        format!(
            "\n[package.metadata.cargo-machete]\nignored = [{}]\n",
            machete_ignored.join(", ")
        )
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
{dep_spec}{serde_line}{mock_lines}
tokio = {{ version = "1", features = ["full"] }}
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
    let function_name = resolve_function_name(e2e_config);
    let module = resolve_module(e2e_config, dep_name);
    let result_var = &e2e_config.call.result_var;
    let has_mock = fixture.needs_mock_server();

    // Tests with a mock server are always async (Axum requires a Tokio runtime).
    let is_async = e2e_config.call.r#async || has_mock;
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
    for arg in &e2e_config.call.args {
        let value = resolve_field(&fixture.input, &arg.field);
        let var_name = &arg.name;
        let (bindings, expr) = render_rust_arg(
            var_name,
            value,
            &arg.arg_type,
            arg.optional,
            &module,
            &fixture.id,
            if has_mock { Some("mock_server.url.as_str()") } else { None },
        );
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
    let path = call_config
        .path
        .as_deref()
        .unwrap_or("/");
    let method = call_config
        .method
        .as_deref()
        .unwrap_or("POST");

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
