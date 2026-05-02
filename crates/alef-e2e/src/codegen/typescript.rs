//! TypeScript e2e test generator using vitest.

use crate::config::E2eConfig;
use crate::escape::{escape_js, expand_fixture_templates, sanitize_filename, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, CallbackAction, Fixture, FixtureGroup, ValidationErrorExpectation};
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::template_versions as tv;
use anyhow::Result;
use heck::ToUpperCamelCase;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;
use super::client;

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
            .unwrap_or_else(|| snake_to_camel(&call.function));
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

        // Determine whether any group has HTTP server test fixtures.
        let has_http_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| f.is_http_test());

        // Detect whether any fixture uses file_path or bytes args — if so we need to
        // chdir to the test_documents directory so relative paths resolve correctly.
        let has_file_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
            let cc = e2e_config.resolve_call(f.call.as_deref());
            cc.args
                .iter()
                .any(|a| a.arg_type == "file_path" || a.arg_type == "bytes")
        });

        // Generate package.json.
        files.push(GeneratedFile {
            path: output_base.join("package.json"),
            content: render_package_json(
                &pkg_name,
                &pkg_path,
                &pkg_version,
                e2e_config.dep_mode,
                has_http_fixtures,
            ),
            generated_header: false,
        });

        // Generate tsconfig.json.
        files.push(GeneratedFile {
            path: output_base.join("tsconfig.json"),
            content: render_tsconfig(),
            generated_header: false,
        });

        // Check if we need global setup (either for client_factory or HTTP tests).
        let needs_global_setup = client_factory.is_some() || has_http_fixtures;

        // Generate vitest.config.ts — include globalSetup and/or setupFiles when needed.
        files.push(GeneratedFile {
            path: output_base.join("vitest.config.ts"),
            content: render_vitest_config(needs_global_setup, has_file_fixtures),
            generated_header: true,
        });

        // Generate globalSetup.ts when needed (for mock server or HTTP tests).
        if needs_global_setup {
            files.push(GeneratedFile {
                path: output_base.join("globalSetup.ts"),
                content: render_global_setup(),
                generated_header: true,
            });
        }

        // Generate setup.ts when file_path args are used, to chdir to test_documents.
        if has_file_fixtures {
            files.push(GeneratedFile {
                path: output_base.join("setup.ts"),
                content: render_file_setup(),
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
                self.language_name(),
                &group.category,
                &active,
                &module_path,
                &pkg_name,
                &function_name,
                &e2e_config.call.args,
                options_type.as_deref(),
                &field_resolver,
                client_factory,
                e2e_config,
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
    has_http_fixtures: bool,
) -> String {
    let dep_value = match dep_mode {
        crate::config::DependencyMode::Registry => pkg_version.to_string(),
        crate::config::DependencyMode::Local => "workspace:*".to_string(),
    };
    let _ = has_http_fixtures; // TODO: add HTTP test deps when http fixtures are present
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
    "vitest": "{vitest}"
  }}
}}
"#,
        vitest = tv::npm::VITEST,
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

fn render_vitest_config(with_global_setup: bool, with_file_setup: bool) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let setup_files_line = if with_file_setup {
        "    setupFiles: ['./setup.ts'],\n"
    } else {
        ""
    };
    if with_global_setup {
        format!(
            r#"{header}import {{ defineConfig }} from 'vitest/config';

export default defineConfig({{
  test: {{
    include: ['tests/**/*.test.ts'],
    globalSetup: './globalSetup.ts',
{setup_files_line}  }},
}});
"#
        )
    } else {
        format!(
            r#"{header}import {{ defineConfig }} from 'vitest/config';

export default defineConfig({{
  test: {{
    include: ['tests/**/*.test.ts'],
{setup_files_line}  }},
}});
"#
        )
    }
}

fn render_file_setup() -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    header
        + r#"import { fileURLToPath } from 'url';
import { dirname, join } from 'path';

// Change to the test_documents directory so that fixture file paths like
// "pdf/fake_memo.pdf" resolve correctly when running vitest from e2e/node/.
// setup.ts lives in e2e/node/; test_documents lives at the repository root,
// two directories up: e2e/node/ -> e2e/ -> repo root -> test_documents/.
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const testDocumentsDir = join(__dirname, '..', '..', 'test_documents');
process.chdir(testDocumentsDir);
"#
}

fn render_global_setup() -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    header
        + r#"import { spawn } from 'child_process';
import { resolve } from 'path';

let serverProcess: any;

// HTTP client wrapper for making requests to mock server
const createApp = (baseUrl: string) => ({
  async request(path: string, init?: RequestInit): Promise<Response> {
    const url = new URL(path, baseUrl);
    return fetch(url.toString(), init);
  },
});

export async function setup() {
  // Mock server binary must be pre-built (e.g. by CI or `cargo build --manifest-path e2e/rust/Cargo.toml --bin mock-server --release`)
  serverProcess = spawn(
    resolve(__dirname, '../rust/target/release/mock-server'),
    [resolve(__dirname, '../../fixtures')],
    { stdio: ['pipe', 'pipe', 'inherit'] }
  );

  const url = await new Promise<string>((resolve, reject) => {
    serverProcess.stdout.on('data', (data: any) => {
      const match = data.toString().match(/MOCK_SERVER_URL=(.*)/);
      if (match) resolve(match[1].trim());
    });
    setTimeout(() => reject(new Error('Mock server startup timeout')), 30000);
  });

  process.env.MOCK_SERVER_URL = url;

  // Make app available globally to all tests
  (globalThis as any).app = createApp(url);
}

export async function teardown() {
  if (serverProcess) {
    serverProcess.stdin.end();
    serverProcess.kill();
  }
}
"#
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_file(
    lang: &str,
    category: &str,
    fixtures: &[&Fixture],
    module_path: &str,
    pkg_name: &str,
    function_name: &str,
    args: &[crate::config::ArgMapping],
    options_type: Option<&str>,
    field_resolver: &FieldResolver,
    client_factory: Option<&str>,
    e2e_config: &E2eConfig,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let _ = writeln!(out, "import {{ describe, expect, it }} from 'vitest';");

    let has_http_fixtures = fixtures.iter().any(|f| f.is_http_test());
    // Only treat as "has non-HTTP fixtures" when at least one non-HTTP fixture has assertions
    // (fixtures with no assertions are emitted as it.skip stubs that don't call any imports).
    let has_non_http_fixtures = fixtures.iter().any(|f| !f.is_http_test() && !f.assertions.is_empty());

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

    // Build imports for non-HTTP fixtures.
    if has_non_http_fixtures {
        // When using client_factory, import the factory instead of the function.
        let mut imports: Vec<String> = if let Some(factory) = client_factory {
            vec![factory.to_string()]
        } else {
            vec![function_name.to_string()]
        };

        // Also import any additional function names used by per-fixture call overrides.
        for fixture in fixtures.iter().filter(|f| !f.is_http_test()) {
            if fixture.call.is_some() {
                let call_config = e2e_config.resolve_call(fixture.call.as_deref());
                let fixture_fn = resolve_node_function_name(call_config, lang);
                if client_factory.is_none() && !imports.contains(&fixture_fn) {
                    imports.push(fixture_fn);
                }
            }
        }

        // Collect tree helper function names needed by method_result assertions.
        for fixture in fixtures.iter().filter(|f| !f.is_http_test()) {
            for assertion in &fixture.assertions {
                if assertion.assertion_type == "method_result" {
                    if let Some(method_name) = &assertion.method {
                        let helper = ts_method_helper_import(method_name);
                        if let Some(helper_fn) = helper {
                            if !imports.contains(&helper_fn) {
                                imports.push(helper_fn);
                            }
                        }
                    }
                }
            }
        }

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
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "describe('{category}', () => {{");

    for (i, fixture) in fixtures.iter().enumerate() {
        if fixture.is_http_test() {
            render_http_test_case(&mut out, fixture);
        } else {
            render_test_case(
                &mut out,
                lang,
                fixture,
                client_factory,
                options_type,
                field_resolver,
                e2e_config,
            );
        }
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    // Suppress unused variable warning when file has only HTTP fixtures.
    let _ = has_http_fixtures;

    let _ = writeln!(out, "}});");
    out
}

/// Resolve the function name for a call config, applying language-specific overrides.
///
/// Both NAPI-RS (node) and wasm-bindgen (wasm) export Rust snake_case functions as
/// camelCase in JavaScript. When no explicit `function` override is set for `lang`,
/// auto-convert the call config's snake_case function name to camelCase so generated
/// imports match the binding's exports.
fn resolve_node_function_name(call_config: &crate::config::CallConfig, lang: &str) -> String {
    call_config
        .overrides
        .get(lang)
        .and_then(|o| o.function.clone())
        .unwrap_or_else(|| snake_to_camel(&call_config.function))
}

/// Return the package-level helper function name to import for a method_result method,
/// or `None` if the method maps to a property access (no import needed).
fn ts_method_helper_import(method_name: &str) -> Option<String> {
    match method_name {
        "has_error_nodes" => Some("treeHasErrorNodes".to_string()),
        "error_count" | "tree_error_count" => Some("treeErrorCount".to_string()),
        "tree_to_sexp" => Some("treeToSexp".to_string()),
        "contains_node_type" => Some("treeContainsNodeType".to_string()),
        "find_nodes_by_type" => Some("findNodesByType".to_string()),
        "run_query" => Some("runQuery".to_string()),
        // Property accesses (root_child_count, root_node_type, named_children_count)
        // and unknown methods that become `result.method()` don't need extra imports.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// HTTP server test rendering — TestClientRenderer impl + thin driver wrapper
// ---------------------------------------------------------------------------

/// Renderer that emits vitest `it(...)` blocks using the Node.js `fetch` API
/// against the mock server (`process.env.MOCK_SERVER_URL`).
pub(super) struct TypeScriptTestClientRenderer;

impl client::TestClientRenderer for TypeScriptTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "node"
    }

    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let escaped_desc = description.replace('\'', "\\'");
        if let Some(reason) = skip_reason {
            let escaped_reason = reason.replace('\'', "\\'");
            let _ = writeln!(out, "  it.skip('{fn_name}: {escaped_desc}', async () => {{");
            let _ = writeln!(out, "    // skipped: {escaped_reason}");
        } else {
            let _ = writeln!(out, "  it('{fn_name}: {escaped_desc}', async () => {{");
        }
    }

    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "  }});");
    }

    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();
        let fixture_id = escape_js(ctx.path.trim_start_matches("/fixtures/"));

        // Build the init object for `fetch(url, init)`.
        let mut init_entries: Vec<String> = Vec::new();
        init_entries.push(format!("method: '{method}'"));
        // Do not follow redirects — tests that assert on 3xx status codes need the original response.
        init_entries.push("redirect: 'manual'".to_string());

        // Headers
        if !ctx.headers.is_empty() {
            let mut header_pairs: Vec<(&String, &String)> = ctx.headers.iter().collect();
            header_pairs.sort_by_key(|(k, _)| k.as_str());
            let entries: Vec<String> = header_pairs
                .iter()
                .map(|(k, v)| {
                    let expanded_v = expand_fixture_templates(v);
                    format!("      \"{}\": \"{}\"", escape_js(k), escape_js(&expanded_v))
                })
                .collect();
            init_entries.push(format!("headers: {{\n{},\n    }}", entries.join(",\n")));
        }

        // Body
        if let Some(body) = ctx.body {
            let js_body = json_to_js(body);
            init_entries.push(format!("body: JSON.stringify({js_body})"));
        }

        let _ = writeln!(
            out,
            "    const mockUrl = `${{process.env.MOCK_SERVER_URL}}/fixtures/{fixture_id}`;"
        );
        let init_str = init_entries.join(", ");
        let _ = writeln!(
            out,
            "    const {} = await fetch(mockUrl, {{ {init_str} }});",
            ctx.response_var
        );
    }

    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        let _ = writeln!(out, "    expect({response_var}.status).toBe({status});");
    }

    fn render_assert_header(&self, out: &mut String, response_var: &str, name: &str, expected: &str) {
        let escaped_name = escape_js(&name.to_lowercase());
        match expected {
            "<<present>>" => {
                let _ = writeln!(
                    out,
                    "    expect({response_var}.headers.get('{escaped_name}')).not.toBeNull();"
                );
            }
            "<<absent>>" => {
                let _ = writeln!(
                    out,
                    "    expect({response_var}.headers.get('{escaped_name}')).toBeNull();"
                );
            }
            "<<uuid>>" => {
                let _ = writeln!(
                    out,
                    "    expect({response_var}.headers.get('{escaped_name}')).toMatch(/^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$/);"
                );
            }
            exact => {
                let escaped_val = escape_js(exact);
                let _ = writeln!(
                    out,
                    "    expect({response_var}.headers.get('{escaped_name}')).toBe('{escaped_val}');"
                );
            }
        }
    }

    fn render_assert_json_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        if let serde_json::Value::String(s) = expected {
            // Plain-string body: mock server returns raw text, compare as text.
            let escaped = escape_js(s);
            let _ = writeln!(out, "    const text = await {response_var}.text();");
            let _ = writeln!(out, "    expect(text).toBe('{escaped}');");
        } else {
            let js_val = json_to_js(expected);
            let _ = writeln!(out, "    const data = await {response_var}.json();");
            let _ = writeln!(out, "    expect(data).toEqual({js_val});");
        }
    }

    fn render_assert_partial_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        let _ = writeln!(out, "    const data = await {response_var}.json();");
        if let Some(obj) = expected.as_object() {
            for (key, val) in obj {
                let js_key = escape_js(key);
                let js_val = json_to_js(val);
                let _ = writeln!(
                    out,
                    "    expect((data as Record<string, unknown>)['{js_key}']).toEqual({js_val});"
                );
            }
        }
    }

    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        let _ = writeln!(
            out,
            "    const body = await {response_var}.json() as {{ errors?: unknown[] }};"
        );
        let _ = writeln!(out, "    const errors = body.errors ?? [];");
        for ve in errors {
            let loc_js: Vec<String> = ve.loc.iter().map(|s| format!("\"{}\"", escape_js(s))).collect();
            let loc_str = loc_js.join(", ");
            let expanded_msg = expand_fixture_templates(&ve.msg);
            let escaped_msg = escape_js(&expanded_msg);
            let _ = writeln!(
                out,
                "    expect((errors as Array<Record<string, unknown>>).some((e) => JSON.stringify(e[\"loc\"]) === JSON.stringify([{loc_str}]) && String(e[\"msg\"]).includes(\"{escaped_msg}\"))).toBe(true);"
            );
        }
    }
}

/// Render a vitest `it` block for an HTTP server fixture.
///
/// Delegates to the shared [`client::http_call::render_http_test`] driver via
/// [`TypeScriptTestClientRenderer`]. HTTP 101 (WebSocket upgrade) fixtures are
/// emitted as `it.skip` stubs before reaching the driver because `fetch` cannot
/// handle upgrade responses.
fn render_http_test_case(out: &mut String, fixture: &Fixture) {
    let Some(http) = &fixture.http else {
        return;
    };

    // HTTP 101 (WebSocket upgrade) — fetch cannot handle upgrade responses.
    if http.expected_response.status_code == 101 {
        let test_name = sanitize_ident(&fixture.id);
        let description = fixture.description.replace('\'', "\\'");
        let _ = writeln!(out, "  it.skip('{test_name}: {description}', async () => {{");
        let _ = writeln!(out, "    // HTTP 101 WebSocket upgrade cannot be tested via fetch");
        let _ = writeln!(out, "  }});");
        return;
    }

    client::http_call::render_http_test(out, &TypeScriptTestClientRenderer, fixture);
}

// ---------------------------------------------------------------------------
// Function-call test rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_test_case(
    out: &mut String,
    lang: &str,
    fixture: &Fixture,
    client_factory: Option<&str>,
    options_type: Option<&str>,
    field_resolver: &FieldResolver,
    e2e_config: &E2eConfig,
) {
    // Resolve per-fixture call config (supports `"call": "parse"` overrides in fixtures).
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let function_name = resolve_node_function_name(call_config, lang);
    let result_var = &call_config.result_var;
    let is_async = call_config.r#async;
    let args = &call_config.args;

    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.replace('\'', "\\'");
    let async_kw = if is_async { "async " } else { "" };
    let await_kw = if is_async { "await " } else { "" };

    // Build the call expression — either `client.method(args)` or `method(args)`
    let (mut setup_lines, args_str) = build_args_and_setup(&fixture.input, args, options_type, &fixture.id);

    // Build visitor if present and add to setup
    let mut visitor_arg = String::new();
    if let Some(visitor_spec) = &fixture.visitor {
        visitor_arg = build_typescript_visitor(&mut setup_lines, visitor_spec);
    }

    let final_args = if visitor_arg.is_empty() {
        args_str
    } else if args_str.is_empty() {
        format!("{{ visitor: {visitor_arg} }}")
    } else {
        format!("{args_str}, {{ visitor: {visitor_arg} }}")
    };

    let call_expr = if client_factory.is_some() {
        format!("client.{function_name}({final_args})")
    } else {
        format!("{function_name}({final_args})")
    };

    // Build the base_url expression for mock server
    let base_url_expr = format!("`${{process.env.MOCK_SERVER_URL}}/fixtures/{}`", fixture.id);

    // Check if this is an error-expecting test.
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Skip tests with no assertions — they would call a stub function that may not exist.
    if fixture.assertions.is_empty() {
        let _ = writeln!(out, "  it.skip('{test_name}: {description}', async () => {{");
        let _ = writeln!(out, "    // no assertions configured for this fixture in node e2e");
        let _ = writeln!(out, "  }});");
        return;
    }

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
        // A2: skip not_error assertions when returns_result=false (non-Result calls don't return errors).
        if assertion.assertion_type == "not_error" && !call_config.returns_result {
            continue;
        }
        render_assertion(out, assertion, result_var, field_resolver);
    }

    let _ = writeln!(out, "  }});");
}

/// Check whether any arg at index `idx` or later has a non-null value in `input`.
fn has_later_arg_value(args: &[crate::config::ArgMapping], from_idx: usize, input: &serde_json::Value) -> bool {
    args[from_idx..].iter().any(|arg| {
        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = if field == "input" {
            Some(input)
        } else {
            input.get(field)
        };
        !matches!(val, None | Some(serde_json::Value::Null))
    })
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

    for (idx, arg) in args.iter().enumerate() {
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
                // Optional arg with no fixture value — check if any later arg has a value.
                // If so, emit `undefined` as a placeholder to preserve positional order.
                if has_later_arg_value(args, idx + 1, input) {
                    parts.push("undefined".to_string());
                }
                // Otherwise skip entirely (trailing optional args need no placeholder).
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
                // For json_object args, NAPI-RS bindings use camelCase for JS field names,
                // so convert snake_case fixture keys to camelCase before passing.
                if arg.arg_type == "json_object" {
                    if let Some(opts_type) = options_type {
                        parts.push(format!("{} as {opts_type}", json_to_js_camel(v)));
                    } else {
                        parts.push(json_to_js_camel(v));
                    }
                    continue;
                }
                parts.push(json_to_js(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
}

fn render_assertion(out: &mut String, assertion: &Assertion, result_var: &str, field_resolver: &FieldResolver) {
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct property accesses on the result.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content" => {
                let pred = format!("({result_var}.chunks ?? []).every((c: {{ content?: string }}) => !!c.content)");
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "    expect({pred}).toBe(true);");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "    expect({pred}).toBe(false);");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "    // skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            "chunks_have_embeddings" => {
                let pred = format!(
                    "({result_var}.chunks ?? []).every((c: {{ embedding?: number[] }}) => c.embedding != null && c.embedding.length > 0)"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "    expect({pred}).toBe(true);");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "    expect({pred}).toBe(false);");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "    // skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            // ---- EmbedResponse virtual fields ----
            // embed_texts returns number[][] in TypeScript — no wrapper object.
            // result_var is the embedding matrix; use it directly.
            "embeddings" => {
                match assertion.assertion_type.as_str() {
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            let js_val = json_to_js(val);
                            let _ = writeln!(out, "    expect({result_var}.length).toBe({js_val});");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let js_val = json_to_js(val);
                            let _ = writeln!(out, "    expect({result_var}.length).toBeGreaterThanOrEqual({js_val});");
                        }
                    }
                    "not_empty" => {
                        let _ = writeln!(out, "    expect({result_var}.length).toBeGreaterThan(0);");
                    }
                    "is_empty" => {
                        let _ = writeln!(out, "    expect({result_var}.length).toBe(0);");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "    // skipped: unsupported assertion type on synthetic field 'embeddings'"
                        );
                    }
                }
                return;
            }
            "embedding_dimensions" => {
                let expr = format!("({result_var}.length > 0 ? {result_var}[0].length : 0)");
                match assertion.assertion_type.as_str() {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let js_val = json_to_js(val);
                            let _ = writeln!(out, "    expect({expr}).toBe({js_val});");
                        }
                    }
                    "greater_than" => {
                        if let Some(val) = &assertion.value {
                            let js_val = json_to_js(val);
                            let _ = writeln!(out, "    expect({expr}).toBeGreaterThan({js_val});");
                        }
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "    // skipped: unsupported assertion type on synthetic field 'embedding_dimensions'"
                        );
                    }
                }
                return;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                let pred = match f.as_str() {
                    "embeddings_valid" => {
                        format!("{result_var}.every((e: number[]) => e.length > 0)")
                    }
                    "embeddings_finite" => {
                        format!("{result_var}.every((e: number[]) => e.every((v: number) => isFinite(v)))")
                    }
                    "embeddings_non_zero" => {
                        format!("{result_var}.every((e: number[]) => e.some((v: number) => v !== 0))")
                    }
                    "embeddings_normalized" => {
                        format!(
                            "{result_var}.every((e: number[]) => {{ const n = e.reduce((s: number, v: number) => s + v * v, 0); return Math.abs(n - 1.0) < 1e-3; }})"
                        )
                    }
                    _ => unreachable!(),
                };
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "    expect({pred}).toBe(true);");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "    expect({pred}).toBe(false);");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "    // skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            // ---- keywords / keywords_count ----
            // Node JsExtractionResult does not expose extracted_keywords; skip.
            "keywords" | "keywords_count" => {
                let _ = writeln!(
                    out,
                    "    // skipped: field '{f}' not available on Node JsExtractionResult"
                );
                return;
            }
            _ => {}
        }
    }

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
        "is_false" => {
            let _ = writeln!(out, "    expect({field_expr}).toBe(false);");
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                let call_expr = build_ts_method_call(result_var, method_name, assertion.args.as_ref());
                let check = assertion.check.as_deref().unwrap_or("is_true");
                match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let js_val = json_to_js(val);
                            let _ = writeln!(out, "    expect({call_expr}).toBe({js_val});");
                        }
                    }
                    "is_true" => {
                        let _ = writeln!(out, "    expect({call_expr}).toBe(true);");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "    expect({call_expr}).toBe(false);");
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(out, "    expect({call_expr}).toBeGreaterThanOrEqual({n});");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(out, "    expect({call_expr}.length).toBeGreaterThanOrEqual({n});");
                        }
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            let js_val = json_to_js(val);
                            let _ = writeln!(out, "    expect({call_expr}).toContain({js_val});");
                        }
                    }
                    "is_error" => {
                        let _ = writeln!(out, "    expect(() => {{ {call_expr}; }}).toThrow();");
                    }
                    other_check => {
                        panic!("TypeScript e2e generator: unsupported method_result check type: {other_check}");
                    }
                }
            } else {
                panic!("TypeScript e2e generator: method_result assertion missing 'method' field");
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    expect({field_expr}.length).toBeGreaterThanOrEqual({n});");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    expect({field_expr}.length).toBeLessThanOrEqual({n});");
                }
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let _ = writeln!(out, "    expect({field_expr}.endsWith({js_val})).toBe(true);");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                if let Some(pattern) = expected.as_str() {
                    let _ = writeln!(out, "    expect({field_expr}).toMatch(/{pattern}/);");
                }
            }
        }
        "not_error" => {
            // No-op — if we got here, the call succeeded (it would have thrown).
        }
        "error" => {
            // Handled at the test level (early return above).
        }
        other => {
            panic!("TypeScript e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Build a TypeScript call expression for a method_result assertion on a tree-sitter Tree.
/// Maps method names to the appropriate TypeScript function calls or property accesses.
fn build_ts_method_call(result_var: &str, method_name: &str, args: Option<&serde_json::Value>) -> String {
    match method_name {
        "root_child_count" => format!("{result_var}.rootNode.childCount"),
        "root_node_type" => format!("{result_var}.rootNode.type"),
        "named_children_count" => format!("{result_var}.rootNode.namedChildCount"),
        "has_error_nodes" => format!("treeHasErrorNodes({result_var})"),
        "error_count" | "tree_error_count" => format!("treeErrorCount({result_var})"),
        "tree_to_sexp" => format!("treeToSexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("treeContainsNodeType({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("findNodesByType({result_var}, \"{node_type}\")")
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
            format!("runQuery({result_var}, \"{language}\", \"{query_source}\", source)")
        }
        _ => {
            if let Some(args_val) = args {
                let arg_str = args_val
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .map(|(k, v)| format!("{}: {}", k, json_to_js(v)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                format!("{result_var}.{method_name}({arg_str})")
            } else {
                format!("{result_var}.{method_name}()")
            }
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
        serde_json::Value::String(s) => {
            let expanded = expand_fixture_templates(s);
            format!("\"{}\"", escape_js(&expanded))
        }
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => {
            // For integers outside JS safe range, emit as string to avoid precision loss.
            if let Some(i) = n.as_i64() {
                if !(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&i) {
                    return format!("Number(\"{i}\")");
                }
            }
            if let Some(u) = n.as_u64() {
                if u > 9_007_199_254_740_991 {
                    return format!("Number(\"{u}\")");
                }
            }
            n.to_string()
        }
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

// ---------------------------------------------------------------------------
// Visitor generation
// ---------------------------------------------------------------------------

/// Build a TypeScript visitor object and add setup line. Returns the visitor variable name.
fn build_typescript_visitor(setup_lines: &mut Vec<String>, visitor_spec: &crate::fixture::VisitorSpec) -> String {
    use std::fmt::Write as FmtWrite;
    let mut visitor_obj = String::new();
    let _ = writeln!(visitor_obj, "{{");
    for (method_name, action) in &visitor_spec.callbacks {
        emit_typescript_visitor_method(&mut visitor_obj, method_name, action);
    }
    let _ = writeln!(visitor_obj, "    }}");

    setup_lines.push(format!("const _testVisitor = {visitor_obj}"));
    "_testVisitor".to_string()
}

/// Emit a TypeScript visitor method for a callback action.
fn emit_typescript_visitor_method(out: &mut String, method_name: &str, action: &CallbackAction) {
    use std::fmt::Write as FmtWrite;

    let camel_method = to_camel_case(method_name);
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
        "visit_table_row" => "ctx, cells, isHeader",
        "visit_custom_element" => "ctx, tagName, html",
        "visit_form" => "ctx, actionUrl, method",
        "visit_input" => "ctx, inputType, name, value",
        "visit_audio" | "visit_video" | "visit_iframe" => "ctx, src",
        "visit_details" => "ctx, isOpen",
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => "ctx, output",
        "visit_list_start" => "ctx, ordered",
        "visit_list_end" => "ctx, ordered, output",
        _ => "ctx",
    };

    let _ = writeln!(
        out,
        "    {camel_method}({params}): string | {{{{ custom: string }}}} {{"
    );
    match action {
        CallbackAction::Skip => {
            let _ = writeln!(out, "        return \"skip\";");
        }
        CallbackAction::Continue => {
            let _ = writeln!(out, "        return \"continue\";");
        }
        CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "        return \"preserve_html\";");
        }
        CallbackAction::Custom { output } => {
            let escaped = escape_js(output);
            let _ = writeln!(out, "        return {{ custom: {escaped} }};");
        }
        CallbackAction::CustomTemplate { template } => {
            let _ = writeln!(out, "        return {{ custom: `{template}` }};");
        }
    }
    let _ = writeln!(out, "    }},");
}

/// Convert snake_case to camelCase for method names.
fn to_camel_case(snake: &str) -> String {
    use heck::ToLowerCamelCase;
    snake.to_lower_camel_case()
}
