//! Elixir e2e test generator using ExUnit.

use crate::config::E2eConfig;
use crate::escape::{escape_elixir, sanitize_filename, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, CallbackAction, Fixture, FixtureGroup, HttpFixture, ValidationErrorExpectation};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::template_versions as tv;
use anyhow::Result;
use heck::ToSnakeCase;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;
use super::client;

/// Elixir e2e code generator.
pub struct ElixirCodegen;

impl E2eCodegen for ElixirCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        _type_defs: &[alef_core::ir::TypeDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        // Resolve call config with overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let raw_module = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        // Convert module path to Elixir PascalCase if it looks like snake_case
        // (e.g., "html_to_markdown" -> "HtmlToMarkdown").
        // If the override already contains "." (e.g., "Elixir.HtmlToMarkdown"), use as-is.
        let module_path = if raw_module.contains('.') || raw_module.chars().next().is_some_and(|c| c.is_uppercase()) {
            raw_module.clone()
        } else {
            elixir_module_name(&raw_module)
        };
        let base_function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        // Elixir facade exports async variants with `_async` suffix when the call is async.
        // Append the suffix only if not already present and the function isn't a streaming
        // entry-point — streaming wrappers (e.g. `defaultclient_chat_stream`) drive the
        // FFI iterator handle and aren't async-callable in the OpenAI sense.
        let function_name =
            if call.r#async && !base_function_name.ends_with("_async") && !base_function_name.ends_with("_stream") {
                format!("{base_function_name}_async")
            } else {
                base_function_name
            };
        let options_type = overrides.and_then(|o| o.options_type.clone());
        let options_default_fn = overrides.and_then(|o| o.options_via.clone());
        let empty_enum_fields = HashMap::new();
        let enum_fields = overrides.map(|o| &o.enum_fields).unwrap_or(&empty_enum_fields);
        let handle_struct_type = overrides.and_then(|o| o.handle_struct_type.clone());
        let empty_atom_fields = std::collections::HashSet::new();
        let handle_atom_list_fields = overrides
            .map(|o| &o.handle_atom_list_fields)
            .unwrap_or(&empty_atom_fields);
        let result_var = &call.result_var;

        // Check if any fixture in any group is an HTTP test.
        let has_http_tests = groups.iter().any(|g| g.fixtures.iter().any(|f| f.is_http_test()));
        let has_nif_tests = groups.iter().any(|g| g.fixtures.iter().any(|f| !f.is_http_test()));
        // Check if any fixture needs the mock server (either via http or mock_response or client_factory).
        let has_mock_server_tests = groups.iter().any(|g| {
            g.fixtures.iter().any(|f| {
                if f.needs_mock_server() {
                    return true;
                }
                let cc = e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.input);
                let elixir_override = cc
                    .overrides
                    .get("elixir")
                    .or_else(|| e2e_config.call.overrides.get("elixir"));
                elixir_override.and_then(|o| o.client_factory.as_deref()).is_some()
            })
        });

        // Resolve package reference (path or version) for the NIF dependency.
        let pkg_ref = e2e_config.resolve_package(lang);
        let pkg_path = if has_nif_tests {
            pkg_ref
                .as_ref()
                .and_then(|p| p.path.as_deref())
                .unwrap_or("../../packages/elixir")
        } else {
            ""
        };

        // Generate mix.exs. The dep atom must match the binding package's
        // mix `app:` value, not the crate name. Use the configured
        // `[elixir].app_name` (the same source the package's own mix.exs
        // uses); fall back to the crate name only when unset. Without this,
        // mix's path-dep resolution silently misroutes — the path-dep's
        // own deps (notably `:rustler_precompiled`) never load during its
        // compilation and the parent build fails with `RustlerPrecompiled
        // is not loaded`.
        let pkg_atom = config.elixir_app_name();
        files.push(GeneratedFile {
            path: output_base.join("mix.exs"),
            content: render_mix_exs(&pkg_atom, pkg_path, e2e_config.dep_mode, has_http_tests, has_nif_tests),
            generated_header: false,
        });

        // Generate lib/e2e_elixir.ex — required so the mix project compiles.
        files.push(GeneratedFile {
            path: output_base.join("lib").join("e2e_elixir.ex"),
            content: "defmodule E2eElixir do\n  @moduledoc false\nend\n".to_string(),
            generated_header: false,
        });

        // Generate test_helper.exs.
        files.push(GeneratedFile {
            path: output_base.join("test").join("test_helper.exs"),
            content: render_test_helper(has_http_tests || has_mock_server_tests),
            generated_header: false,
        });

        // Generate test files per category.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("{}_test.exs", sanitize_filename(&group.category));
            let field_resolver = FieldResolver::new(
                &e2e_config.fields,
                &e2e_config.fields_optional,
                &e2e_config.result_fields,
                &e2e_config.fields_array,
                &std::collections::HashSet::new(),
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
                options_type.as_deref(),
                options_default_fn.as_deref(),
                enum_fields,
                handle_struct_type.as_deref(),
                handle_atom_list_fields,
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
        "elixir"
    }
}

fn render_test_helper(has_http_tests: bool) -> String {
    if has_http_tests {
        r#"ExUnit.start()

# Spawn mock-server binary and set MOCK_SERVER_URL for all tests.
mock_server_bin = Path.expand("../../rust/target/release/mock-server", __DIR__)
fixtures_dir = Path.expand("../../../fixtures", __DIR__)

if File.exists?(mock_server_bin) do
  port = Port.open({:spawn_executable, mock_server_bin}, [
    :binary,
    # Use a large line buffer (default 1024 truncates `MOCK_SERVERS={...}` lines for
    # fixture sets with many host-root routes, splitting them into `:noeol` chunks
    # that the prefix-match clauses below would never see).
    {:line, 65_536},
    args: [fixtures_dir]
  ])
  # Read startup lines: MOCK_SERVER_URL= then MOCK_SERVERS= (always emitted, possibly `{}`).
  # The standalone mock-server prints noisy stderr lines BEFORE the stdout sentinels;
  # selective receive ignores anything that doesn't match the two prefix patterns.
  # Each iteration only halts after the MOCK_SERVERS= line is processed.
  {url, _} =
    Enum.reduce_while(1..16, {nil, port}, fn _, {url_acc, p} ->
      receive do
        {^p, {:data, {:eol, "MOCK_SERVER_URL=" <> u}}} ->
          {:cont, {u, p}}

        {^p, {:data, {:eol, "MOCK_SERVERS=" <> json_val}}} ->
          System.put_env("MOCK_SERVERS", json_val)
          case Jason.decode(json_val) do
            {:ok, servers} ->
              Enum.each(servers, fn {fid, furl} ->
                System.put_env("MOCK_SERVER_#{String.upcase(fid)}", furl)
              end)

            _ ->
              :ok
          end

          {:halt, {url_acc, p}}
      after
        30_000 ->
          raise "mock-server startup timeout"
      end
    end)

  if url != nil do
    System.put_env("MOCK_SERVER_URL", url)
  end
end
"#
        .to_string()
    } else {
        "ExUnit.start()\n".to_string()
    }
}

fn render_mix_exs(
    pkg_name: &str,
    pkg_path: &str,
    dep_mode: crate::config::DependencyMode,
    has_http_tests: bool,
    has_nif_tests: bool,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "defmodule E2eElixir.MixProject do");
    let _ = writeln!(out, "  use Mix.Project");
    let _ = writeln!(out);
    let _ = writeln!(out, "  def project do");
    let _ = writeln!(out, "    [");
    let _ = writeln!(out, "      app: :e2e_elixir,");
    let _ = writeln!(out, "      version: \"0.1.0\",");
    let _ = writeln!(out, "      elixir: \"~> 1.14\",");
    let _ = writeln!(out, "      deps: deps()");
    let _ = writeln!(out, "    ]");
    let _ = writeln!(out, "  end");
    let _ = writeln!(out);
    let _ = writeln!(out, "  defp deps do");
    let _ = writeln!(out, "    [");

    // Build the list of deps, then join with commas to avoid double-commas.
    let mut deps: Vec<String> = Vec::new();

    // Add the binding NIF dependency when there are non-HTTP tests.
    if has_nif_tests && !pkg_path.is_empty() {
        let pkg_atom = pkg_name;
        let nif_dep = match dep_mode {
            crate::config::DependencyMode::Local => {
                format!("      {{:{pkg_atom}, path: \"{pkg_path}\"}}")
            }
            crate::config::DependencyMode::Registry => {
                // Registry mode: pkg_path is repurposed as the version string.
                format!("      {{:{pkg_atom}, \"{pkg_path}\"}}")
            }
        };
        deps.push(nif_dep);
        // rustler_precompiled provides the precompiled NIF loader.
        deps.push(format!(
            "      {{:rustler_precompiled, \"{rp}\"}}",
            rp = tv::hex::RUSTLER_PRECOMPILED
        ));
        // rustler must be a direct, non-optional dep in the consumer project for
        // `force_build: Mix.env() in [:test, :dev]` to actually fetch the rustler hex
        // package. With `optional: true` mix omits it when no other dep declares it as
        // required, breaking the build-from-source path used by the e2e suite.
        deps.push(format!(
            "      {{:rustler, \"{rustler}\", runtime: false}}",
            rustler = tv::hex::RUSTLER
        ));
    }

    // Add Req + Jason for HTTP testing.
    if has_http_tests {
        deps.push(format!("      {{:req, \"{req}\"}}", req = tv::hex::REQ));
        deps.push(format!("      {{:jason, \"{jason}\"}}", jason = tv::hex::JASON));
    }

    let _ = writeln!(out, "{}", deps.join(",\n"));
    let _ = writeln!(out, "    ]");
    let _ = writeln!(out, "  end");
    let _ = writeln!(out, "end");
    out
}

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    module_path: &str,
    function_name: &str,
    result_var: &str,
    args: &[crate::config::ArgMapping],
    field_resolver: &FieldResolver,
    options_type: Option<&str>,
    options_default_fn: Option<&str>,
    enum_fields: &HashMap<String, String>,
    handle_struct_type: Option<&str>,
    handle_atom_list_fields: &std::collections::HashSet<String>,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "# E2e tests for category: {category}");
    let _ = writeln!(out, "defmodule E2e.{}Test do", elixir_module_name(category));

    // Add client helper when there are HTTP fixtures in this group.
    let has_http = fixtures.iter().any(|f| f.is_http_test());

    // Use async: false for NIF tests — concurrent Tokio runtimes created by DirtyCpu NIFs
    // on ARM64 macOS cause SIGBUS when tests run in parallel. HTTP-only tests can stay async.
    let async_flag = if has_http { "true" } else { "false" };
    let _ = writeln!(out, "  use ExUnit.Case, async: {async_flag}");

    if has_http {
        let _ = writeln!(out);
        let _ = writeln!(out, "  defp mock_server_url do");
        let _ = writeln!(
            out,
            "    System.get_env(\"MOCK_SERVER_URL\") || \"http://localhost:8080\""
        );
        let _ = writeln!(out, "  end");
    }

    // Emit a shared helper for array field contains assertions — extracts string
    // representations from each item's attributes so String.contains? works on struct lists.
    let has_array_contains = fixtures.iter().any(|fixture| {
        fixture.assertions.iter().any(|a| {
            matches!(a.assertion_type.as_str(), "contains" | "contains_all" | "not_contains")
                && a.field
                    .as_deref()
                    .is_some_and(|f| !f.is_empty() && field_resolver.is_array(field_resolver.resolve(f)))
        })
    });
    if has_array_contains {
        let _ = writeln!(out);
        let _ = writeln!(out, "  defp alef_e2e_item_texts(item) when is_binary(item), do: [item]");
        let _ = writeln!(out, "  defp alef_e2e_item_texts(item) do");
        let _ = writeln!(out, "    [:kind, :name, :signature, :path, :alias, :text, :source]");
        let _ = writeln!(out, "    |> Enum.filter(&Map.has_key?(item, &1))");
        let _ = writeln!(out, "    |> Enum.flat_map(fn attr ->");
        let _ = writeln!(out, "      case Map.get(item, attr) do");
        let _ = writeln!(out, "        nil -> []");
        let _ = writeln!(
            out,
            "        atom when is_atom(atom) -> [atom |> to_string() |> String.capitalize()]"
        );
        let _ = writeln!(out, "        str -> [to_string(str)]");
        let _ = writeln!(out, "      end");
        let _ = writeln!(out, "    end)");
        let _ = writeln!(out, "  end");
    }

    let _ = writeln!(out);

    for (i, fixture) in fixtures.iter().enumerate() {
        if let Some(http) = &fixture.http {
            render_http_test_case(&mut out, fixture, http);
        } else {
            render_test_case(
                &mut out,
                fixture,
                e2e_config,
                module_path,
                function_name,
                result_var,
                args,
                field_resolver,
                options_type,
                options_default_fn,
                enum_fields,
                handle_struct_type,
                handle_atom_list_fields,
            );
        }
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    let _ = writeln!(out, "end");
    out
}

// ---------------------------------------------------------------------------
// HTTP test rendering
// ---------------------------------------------------------------------------

/// HTTP methods that Finch (Req's underlying HTTP client) does not support.
/// Tests using these methods are emitted with `@tag :skip` so they don't fail.
const FINCH_UNSUPPORTED_METHODS: &[&str] = &["TRACE", "CONNECT"];

/// HTTP methods that Req exposes as convenience functions.
/// All others must be called via `Req.request(method: :METHOD, ...)`.
const REQ_CONVENIENCE_METHODS: &[&str] = &["get", "post", "put", "patch", "delete", "head"];

/// Thin renderer that emits ExUnit `describe` + `test` blocks targeting a mock
/// server via `Req`. Satisfies [`client::TestClientRenderer`] so the shared
/// [`client::http_call::render_http_test`] driver drives the call sequence.
struct ElixirTestClientRenderer<'a> {
    /// The fixture id is needed in [`render_call`] to build the mock server URL
    /// (`mock_server_url()/fixtures/<id>`).
    fixture_id: &'a str,
    /// Expected response status, needed to disable Req's redirect-following for 3xx.
    expected_status: u16,
}

impl<'a> client::TestClientRenderer for ElixirTestClientRenderer<'a> {
    fn language_name(&self) -> &'static str {
        "elixir"
    }

    /// Emit `describe "{fn_name}" do` + inner `test "METHOD PATH - description" do`.
    ///
    /// When `skip_reason` is `Some`, emit `@tag :skip` before the test block so
    /// ExUnit skips it; the shared driver short-circuits before emitting any
    /// assertions and then calls `render_test_close` for symmetry.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let escaped_description = description.replace('"', "\\\"");
        let _ = writeln!(out, "  describe \"{fn_name}\" do");
        if skip_reason.is_some() {
            let _ = writeln!(out, "    @tag :skip");
        }
        let _ = writeln!(out, "    test \"{escaped_description}\" do");
    }

    /// Close the inner `test` block and the outer `describe` block.
    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
    }

    /// Emit a `Req` request to the mock server using `mock_server_url()/fixtures/<id>`.
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_lowercase();
        let mut opts: Vec<String> = Vec::new();

        if let Some(body) = ctx.body {
            let elixir_val = json_to_elixir(body);
            opts.push(format!("json: {elixir_val}"));
        }

        if !ctx.headers.is_empty() {
            let header_pairs: Vec<String> = ctx
                .headers
                .iter()
                .map(|(k, v)| format!("{{\"{}\", \"{}\"}}", escape_elixir(k), escape_elixir(v)))
                .collect();
            opts.push(format!("headers: [{}]", header_pairs.join(", ")));
        }

        if !ctx.cookies.is_empty() {
            let cookie_str = ctx
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            opts.push(format!("headers: [{{\"cookie\", \"{}\"}}]", escape_elixir(&cookie_str)));
        }

        if !ctx.query_params.is_empty() {
            let pairs: Vec<String> = ctx
                .query_params
                .iter()
                .map(|(k, v)| {
                    let val_str = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    format!("{{\"{}\", \"{}\"}}", escape_elixir(k), escape_elixir(&val_str))
                })
                .collect();
            opts.push(format!("params: [{}]", pairs.join(", ")));
        }

        // When the expected response is a redirect (3xx), disable automatic redirect
        // following so the test can assert the redirect status and Location header.
        if (300..400).contains(&self.expected_status) {
            opts.push("redirect: false".to_string());
        }

        let fixture_id = escape_elixir(self.fixture_id);
        let url_expr = format!("\"#{{mock_server_url()}}/fixtures/{fixture_id}\"");

        if REQ_CONVENIENCE_METHODS.contains(&method.as_str()) {
            if opts.is_empty() {
                let _ = writeln!(out, "      {{:ok, response}} = Req.{method}(url: {url_expr})");
            } else {
                let opts_str = opts.join(", ");
                let _ = writeln!(
                    out,
                    "      {{:ok, response}} = Req.{method}(url: {url_expr}, {opts_str})"
                );
            }
        } else {
            opts.insert(0, format!("method: :{method}"));
            opts.insert(1, format!("url: {url_expr}"));
            let opts_str = opts.join(", ");
            let _ = writeln!(out, "      {{:ok, response}} = Req.request({opts_str})");
        }
    }

    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        let _ = writeln!(out, "      assert {response_var}.status == {status}");
    }

    /// Emit a header assertion.
    ///
    /// Handles the special tokens `<<present>>`, `<<absent>>`, `<<uuid>>`.
    /// Skips the `connection` header (hop-by-hop, stripped by Req/Mint).
    fn render_assert_header(&self, out: &mut String, response_var: &str, name: &str, expected: &str) {
        let header_key = name.to_lowercase();
        // Req (via Mint) strips hop-by-hop headers; asserting on them is meaningless.
        if header_key == "connection" {
            return;
        }
        let key_lit = format!("\"{}\"", escape_elixir(&header_key));
        let get_header_expr = format!(
            "Enum.find_value({response_var}.headers, fn {{k, v}} -> if String.downcase(k) == {key_lit}, do: List.first(List.wrap(v)) end)"
        );
        match expected {
            "<<present>>" => {
                let _ = writeln!(out, "      assert {get_header_expr} != nil");
            }
            "<<absent>>" => {
                let _ = writeln!(out, "      assert {get_header_expr} == nil");
            }
            "<<uuid>>" => {
                let var = sanitize_ident(&header_key);
                let _ = writeln!(out, "      header_val_{var} = {get_header_expr}");
                let _ = writeln!(
                    out,
                    "      assert Regex.match?(~r/^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$/i, to_string(header_val_{var}))"
                );
            }
            literal => {
                let val_lit = format!("\"{}\"", escape_elixir(literal));
                let _ = writeln!(out, "      assert {get_header_expr} == {val_lit}");
            }
        }
    }

    /// Emit a full JSON body equality assertion.
    ///
    /// Req auto-decodes `application/json` bodies; when the response body is a
    /// binary (non-JSON content type), decode it with `Jason.decode!` first.
    fn render_assert_json_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        let elixir_val = json_to_elixir(expected);
        match expected {
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                let _ = writeln!(
                    out,
                    "      body_decoded = if is_binary({response_var}.body), do: Jason.decode!({response_var}.body), else: {response_var}.body"
                );
                let _ = writeln!(out, "      assert body_decoded == {elixir_val}");
            }
            _ => {
                let _ = writeln!(out, "      assert {response_var}.body == {elixir_val}");
            }
        }
    }

    /// Emit partial body assertions: one assertion per key in `expected`.
    fn render_assert_partial_body(&self, out: &mut String, response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let _ = writeln!(
                out,
                "      decoded_body = if is_binary({response_var}.body), do: Jason.decode!({response_var}.body), else: {response_var}.body"
            );
            for (key, val) in obj {
                let key_lit = format!("\"{}\"", escape_elixir(key));
                let elixir_val = json_to_elixir(val);
                let _ = writeln!(out, "      assert decoded_body[{key_lit}] == {elixir_val}");
            }
        }
    }

    /// Emit validation-error assertions, checking each expected `msg` appears in
    /// the encoded response body.
    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        for err in errors {
            let msg_lit = format!("\"{}\"", escape_elixir(&err.msg));
            let _ = writeln!(
                out,
                "      assert String.contains?(Jason.encode!({response_var}.body), {msg_lit})"
            );
        }
    }
}

/// Render an ExUnit `describe` + `test` block for an HTTP server test fixture.
///
/// Delegates to [`client::http_call::render_http_test`] after the one
/// Elixir-specific pre-condition: HTTP methods unsupported by Finch (the
/// underlying Req adapter) are emitted with `@tag :skip` directly.
fn render_http_test_case(out: &mut String, fixture: &Fixture, http: &HttpFixture) {
    let method = http.request.method.to_uppercase();

    // Finch does not support TRACE/CONNECT — emit a skipped test stub directly
    // rather than routing through the shared driver, which would assert on the
    // response and fail.
    if FINCH_UNSUPPORTED_METHODS.contains(&method.as_str()) {
        let test_name = sanitize_ident(&fixture.id);
        let test_label = fixture.id.replace('"', "\\\"");
        let path = &http.request.path;
        let _ = writeln!(out, "  describe \"{test_name}\" do");
        let _ = writeln!(out, "    @tag :skip");
        let _ = writeln!(out, "    test \"{method} {path} - {test_label}\" do");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
        return;
    }

    let renderer = ElixirTestClientRenderer {
        fixture_id: &fixture.id,
        expected_status: http.expected_response.status_code,
    };
    client::http_call::render_http_test(out, &renderer, fixture);
}

// ---------------------------------------------------------------------------
// Function-call test rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_test_case(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    default_module_path: &str,
    default_function_name: &str,
    default_result_var: &str,
    args: &[crate::config::ArgMapping],
    field_resolver: &FieldResolver,
    options_type: Option<&str>,
    options_default_fn: Option<&str>,
    enum_fields: &HashMap<String, String>,
    handle_struct_type: Option<&str>,
    handle_atom_list_fields: &std::collections::HashSet<String>,
) {
    let test_name = sanitize_ident(&fixture.id);
    let test_label = fixture.id.replace('"', "\\\"");

    // Non-HTTP non-mock_response fixtures (e.g. AsyncAPI, WebSocket, OpenRPC
    // protocol-only fixtures) cannot be tested via the configured `[e2e.call]`
    // function when the binding does not expose it. Emit a documented `@tag :skip`
    // test so the suite stays compilable. HTTP fixtures dispatch via render_http_test_case
    // and never reach here.
    if fixture.mock_response.is_none() && !fixture_has_elixir_callable(fixture, e2e_config) {
        let _ = writeln!(out, "  describe \"{test_name}\" do");
        let _ = writeln!(out, "    @tag :skip");
        let _ = writeln!(out, "    test \"{test_label}\" do");
        let _ = writeln!(
            out,
            "      # non-HTTP fixture: Elixir binding does not expose a callable for the configured `[e2e.call]` function"
        );
        let _ = writeln!(out, "      :ok");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
        return;
    }

    // Resolve per-fixture call config (falls back to default if fixture.call is None).
    let call_config = e2e_config.resolve_call_for_fixture(fixture.call.as_deref(), &fixture.input);
    let lang = "elixir";
    let call_overrides = call_config.overrides.get(lang);

    // Check if the function is excluded from the Elixir binding (e.g., batch functions
    // that require unsafe NIF tuple marshalling). Emit a skipped test with rationale.
    let base_fn = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    if base_fn.starts_with("batch_extract_") {
        let _ = writeln!(
            out,
            "  describe \"{test_name}\" do",
            test_name = sanitize_ident(&fixture.id)
        );
        let _ = writeln!(out, "    @tag :skip");
        let _ = writeln!(
            out,
            "    test \"{test_label}\" do",
            test_label = fixture.id.replace('"', "\\\"")
        );
        let _ = writeln!(
            out,
            "      # batch functions excluded from Elixir binding: unsafe NIF tuple marshalling"
        );
        let _ = writeln!(out, "      :ok");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
        return;
    }

    // Compute module_path and function_name from the resolved call config,
    // applying Elixir-specific PascalCase conversion.
    let (module_path, function_name, result_var) = if fixture.call.is_some() {
        let raw_module = call_overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call_config.module.clone());
        let resolved_module = if raw_module.contains('.') || raw_module.chars().next().is_some_and(|c| c.is_uppercase())
        {
            raw_module.clone()
        } else {
            elixir_module_name(&raw_module)
        };
        let resolved_fn = if call_config.r#async && !base_fn.ends_with("_async") && !base_fn.ends_with("_stream") {
            format!("{base_fn}_async")
        } else {
            base_fn
        };
        (resolved_module, resolved_fn, call_config.result_var.clone())
    } else {
        (
            default_module_path.to_string(),
            default_function_name.to_string(),
            default_result_var.to_string(),
        )
    };

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    // Validation-category fixtures expect engine creation itself to fail (bad config).
    // Other expects_error fixtures (e.g. error_*) construct a valid engine and expect the
    // *operation under test* to fail. We need different shapes for these two cases.
    let validation_creation_failure = expects_error && fixture.resolved_category() == "validation";

    // When the fixture uses a named call, use the args and options from that call's config.
    let (
        effective_args,
        effective_options_type,
        effective_options_default_fn,
        effective_enum_fields,
        effective_handle_struct_type,
        effective_handle_atom_list_fields,
    );
    let empty_enum_fields_local: HashMap<String, String>;
    let empty_atom_fields_local: std::collections::HashSet<String>;
    let (
        resolved_args,
        resolved_options_type,
        resolved_options_default_fn,
        resolved_enum_fields_ref,
        resolved_handle_struct_type,
        resolved_handle_atom_list_fields_ref,
    ) = if fixture.call.is_some() {
        let co = call_config.overrides.get(lang);
        effective_args = call_config.args.as_slice();
        effective_options_type = co.and_then(|o| o.options_type.as_deref());
        effective_options_default_fn = co.and_then(|o| o.options_via.as_deref());
        empty_enum_fields_local = HashMap::new();
        effective_enum_fields = co.map(|o| &o.enum_fields).unwrap_or(&empty_enum_fields_local);
        effective_handle_struct_type = co.and_then(|o| o.handle_struct_type.as_deref());
        empty_atom_fields_local = std::collections::HashSet::new();
        effective_handle_atom_list_fields = co
            .map(|o| &o.handle_atom_list_fields)
            .unwrap_or(&empty_atom_fields_local);
        (
            effective_args,
            effective_options_type,
            effective_options_default_fn,
            effective_enum_fields,
            effective_handle_struct_type,
            effective_handle_atom_list_fields,
        )
    } else {
        (
            args as &[_],
            options_type,
            options_default_fn,
            enum_fields,
            handle_struct_type,
            handle_atom_list_fields,
        )
    };

    let test_documents_path = e2e_config.test_documents_relative_from(0);
    let (mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        resolved_args,
        &module_path,
        resolved_options_type,
        resolved_options_default_fn,
        resolved_enum_fields_ref,
        fixture,
        resolved_handle_struct_type,
        resolved_handle_atom_list_fields_ref,
        &test_documents_path,
    );

    // Build visitor if present — it will be injected into the options map.
    let visitor_var = fixture
        .visitor
        .as_ref()
        .map(|visitor_spec| build_elixir_visitor(&mut setup_lines, visitor_spec));

    // If we have a visitor and the args contain a nil for options, replace it with a map
    // containing the visitor. The fixture.visitor is already set above.
    let final_args = if let Some(ref visitor_var) = visitor_var {
        // Parse args_str to handle injection properly.
        // Since we're dealing with a 2-arg function (html, options), and options might be nil,
        // we need to inject visitor into the options.
        let parts: Vec<&str> = args_str.split(", ").collect();
        if parts.len() == 2 && parts[1] == "nil" {
            // Replace nil with %{visitor: visitor}
            format!("{}, %{{visitor: {}}}", parts[0], visitor_var)
        } else if parts.len() == 2 {
            // Options is a variable (e.g., "options") — add visitor to it
            setup_lines.push(format!(
                "{} = Map.put({}, :visitor, {})",
                parts[1], parts[1], visitor_var
            ));
            args_str
        } else if parts.len() == 1 {
            // Only HTML provided — create options map with just visitor
            format!("{}, %{{visitor: {}}}", parts[0], visitor_var)
        } else {
            args_str
        }
    } else {
        args_str
    };

    // Client factory: when configured, create a client and pass it as the first argument.
    let client_factory = call_overrides.and_then(|o| o.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get("elixir")
            .and_then(|o| o.client_factory.as_deref())
    });

    // Append per-call extra_args (e.g. trailing `nil` for `list_files(client, query)`)
    // so Elixir matches the binding's positional arity. Mirrors the same override the
    // Ruby/Go/Node codegens already honor.
    let extra_args: Vec<String> = call_overrides.map(|o| o.extra_args.clone()).unwrap_or_default();
    let final_args_with_extras = if extra_args.is_empty() {
        final_args
    } else if final_args.is_empty() {
        extra_args.join(", ")
    } else {
        format!("{final_args}, {}", extra_args.join(", "))
    };

    // Prefix the client variable to the args when client_factory is set.
    let effective_args = if client_factory.is_some() {
        if final_args_with_extras.is_empty() {
            "client".to_string()
        } else {
            format!("client, {final_args_with_extras}")
        }
    } else {
        final_args_with_extras
    };

    // Real-API smoke fixtures (no mock_response, no http) must be env-gated on the
    // configured `env.api_key_var` so absent keys yield a deterministic skip rather
    // than a spurious "no mock route" failure. Mirrors the Python conftest skip.
    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var_opt = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    let needs_api_key_skip = !has_mock && api_key_var_opt.is_some();
    // When the fixture has both a mock and an api_key_var, generate env-fallback code:
    // use the real API when the key is set, otherwise fall back to the mock server.
    let needs_env_fallback = has_mock && api_key_var_opt.is_some();

    let _ = writeln!(out, "  describe \"{test_name}\" do");
    let _ = writeln!(out, "    test \"{test_label}\" do");

    if needs_api_key_skip {
        let api_key_var = api_key_var_opt.unwrap_or("");
        let _ = writeln!(out, "      if System.get_env(\"{api_key_var}\") in [nil, \"\"] do");
        let _ = writeln!(out, "        # {api_key_var} not set — skipping live smoke test");
        let _ = writeln!(out, "        :ok");
        let _ = writeln!(out, "      else");
    }

    // Validation-category fixtures: engine/handle creation itself is expected to fail.
    // Transform the first `{:ok, _} = ...` setup line into `assert {:error, _} = ...`
    // and stop emission there, since the rest of the test body would be unreachable.
    if validation_creation_failure {
        let mut emitted_error_assertion = false;
        for line in &setup_lines {
            if !emitted_error_assertion && line.starts_with("{:ok,") {
                if let Some(rhs) = line.split_once('=').map(|x| x.1) {
                    let rhs = rhs.trim();
                    let _ = writeln!(out, "      assert {{:error, _}} = {rhs}");
                    emitted_error_assertion = true;
                } else {
                    let _ = writeln!(out, "      {line}");
                }
            } else {
                let _ = writeln!(out, "      {line}");
            }
        }
        if !emitted_error_assertion {
            let _ = writeln!(
                out,
                "      assert {{:error, _}} = {module_path}.{function_name}({effective_args})"
            );
        }
        if needs_api_key_skip {
            let _ = writeln!(out, "      end");
        }
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
        return;
    }

    // Non-validation expects_error fixtures (error_*, etc.): engine creation succeeds.
    // Emit setup as-is and assert that the *operation under test* fails. The
    // call body still references `client` (e.g. `defaultclient_chat_async(client, …)`),
    // so we must emit the same `{:ok, client} = create_client(...)` line that the
    // non-error path below uses — without it the generated test fails to compile
    // with `undefined variable "client"`.
    if expects_error {
        for line in &setup_lines {
            let _ = writeln!(out, "      {line}");
        }
        if let Some(factory) = client_factory {
            let fixture_id = &fixture.id;
            let base_url_expr = if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                format!(
                    "(System.get_env(\"{env_key}\") || (System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\")"
                )
            } else {
                format!("(System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\"")
            };
            let _ = writeln!(
                out,
                "      {{:ok, client}} = {module_path}.{factory}(\"test-key\", {base_url_expr})"
            );
        }
        let _ = writeln!(
            out,
            "      assert {{:error, _}} = {module_path}.{function_name}({effective_args})"
        );
        if needs_api_key_skip {
            let _ = writeln!(out, "      end");
        }
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
        return;
    }

    for line in &setup_lines {
        let _ = writeln!(out, "      {line}");
    }

    // Emit client creation when client_factory is configured.
    if let Some(factory) = client_factory {
        let fixture_id = &fixture.id;
        if needs_env_fallback {
            // Fixture has both a mock and an api_key_var: use the real API when the key is
            // set, otherwise fall back to the mock server.
            let api_key_var = api_key_var_opt.unwrap_or("");
            let mock_url_expr = if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                format!(
                    "System.get_env(\"{env_key}\") || (System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\""
                )
            } else {
                format!("(System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\"")
            };
            let _ = writeln!(out, "      api_key_val = System.get_env(\"{api_key_var}\")");
            let _ = writeln!(
                out,
                "      {{api_key_val, base_url_val}} = if api_key_val && api_key_val != \"\" do"
            );
            let _ = writeln!(
                out,
                "        IO.puts(\"{fixture_id}: using real API ({api_key_var} is set)\")"
            );
            let _ = writeln!(out, "        {{api_key_val, nil}}");
            let _ = writeln!(out, "      else");
            let _ = writeln!(
                out,
                "        IO.puts(\"{fixture_id}: using mock server ({api_key_var} not set)\")"
            );
            let _ = writeln!(out, "        {{nil, {mock_url_expr}}}");
            let _ = writeln!(out, "      end");
            let _ = writeln!(
                out,
                "      {{:ok, client}} = {module_path}.{factory}(api_key_val, base_url_val)"
            );
        } else {
            let base_url_expr = if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                format!(
                    "(System.get_env(\"{env_key}\") || (System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\")"
                )
            } else {
                format!("(System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\"")
            };
            let _ = writeln!(
                out,
                "      {{:ok, client}} = {module_path}.{factory}(\"test-key\", {base_url_expr})"
            );
        }
    }

    // Use returns_result from the Elixir override if present, otherwise from base config
    let returns_result = call_overrides
        .and_then(|o| o.returns_result)
        .unwrap_or(call_config.returns_result || client_factory.is_some());

    // Some calls (e.g. speech, file_content) return raw bytes rather than a struct.
    // When the call is marked `result_is_simple`, treat the bound `result` variable as
    // the value itself so assertions on a logical "audio"/"content" field map to the
    // bare binary instead of a struct accessor that doesn't exist.
    let result_is_simple = call_config.result_is_simple || call_overrides.is_some_and(|o| o.result_is_simple);

    if returns_result {
        let _ = writeln!(
            out,
            "      {{:ok, {result_var}}} = {module_path}.{function_name}({effective_args})"
        );
    } else {
        // Non-Result function returns value directly (e.g., bool, String).
        let _ = writeln!(
            out,
            "      {result_var} = {module_path}.{function_name}({effective_args})"
        );
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            &result_var,
            field_resolver,
            &module_path,
            &e2e_config.fields_enum,
            resolved_enum_fields_ref,
            result_is_simple,
        );
    }

    if needs_api_key_skip {
        let _ = writeln!(out, "      end");
    }
    let _ = writeln!(out, "    end");
    let _ = writeln!(out, "  end");
}

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// Returns `(setup_lines, args_string)`.
#[allow(clippy::too_many_arguments)]
/// Emit Elixir batch item map constructors for BatchBytesItem or BatchFileItem arrays.
fn emit_elixir_batch_item_array(arr: &serde_json::Value, elem_type: &str) -> String {
    if let Some(items) = arr.as_array() {
        let item_strs: Vec<String> = items
            .iter()
            .filter_map(|item| {
                if let Some(obj) = item.as_object() {
                    match elem_type {
                        "BatchBytesItem" => {
                            let content = obj.get("content").and_then(|v| v.as_array());
                            let mime_type = obj.get("mime_type").and_then(|v| v.as_str()).unwrap_or("text/plain");
                            let content_code = if let Some(arr) = content {
                                let bytes: Vec<String> =
                                    arr.iter().filter_map(|v| v.as_u64().map(|n| n.to_string())).collect();
                                format!("<<{}>>", bytes.join(", "))
                            } else {
                                "<<>>".to_string()
                            };
                            Some(format!(
                                "%BatchBytesItem{{content: {}, mime_type: \"{}\"}}",
                                content_code, mime_type
                            ))
                        }
                        "BatchFileItem" => {
                            let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or("");
                            Some(format!("%BatchFileItem{{path: \"{}\"}}", path))
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .collect();
        format!("[{}]", item_strs.join(", "))
    } else {
        "[]".to_string()
    }
}

#[allow(clippy::too_many_arguments)]
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    module_path: &str,
    options_type: Option<&str>,
    options_default_fn: Option<&str>,
    enum_fields: &HashMap<String, String>,
    fixture: &crate::fixture::Fixture,
    _handle_struct_type: Option<&str>,
    _handle_atom_list_fields: &std::collections::HashSet<String>,
    test_documents_path: &str,
) -> (Vec<String>, String) {
    let fixture_id = &fixture.id;
    if args.is_empty() {
        // No args config: pass the whole input only when it's non-empty.
        // Functions with no parameters (e.g. language_count) have empty input
        // and must be called with no arguments — not with `%{}`.
        let is_empty_input = match input {
            serde_json::Value::Null => true,
            serde_json::Value::Object(m) => m.is_empty(),
            _ => false,
        };
        if is_empty_input {
            return (Vec::new(), String::new());
        }
        return (Vec::new(), json_to_elixir(input));
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "mock_url" {
            if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                setup_lines.push(format!(
                    "{} = System.get_env(\"{env_key}\") || (System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\"",
                    arg.name,
                ));
            } else {
                setup_lines.push(format!(
                    "{} = (System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\"",
                    arg.name,
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "handle" {
            // Generate a create_{name} call using {:ok, name} = ... pattern.
            // The NIF now accepts config as an optional JSON string (not a NifStruct/NifMap)
            // so that partial maps work: serde_json::from_str respects #[serde(default)].
            let constructor_name = format!("create_{}", arg.name.to_snake_case());
            let config_value = if arg.field == "input" {
                input
            } else {
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                input.get(field).unwrap_or(&serde_json::Value::Null)
            };
            let name = &arg.name;
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("{{:ok, {name}}} = {module_path}.{constructor_name}(nil)"));
            } else {
                // Serialize the config map to a JSON string with Jason so that Rust can
                // deserialize it with serde_json and apply field defaults for missing keys.
                let json_str = serde_json::to_string(config_value).unwrap_or_else(|_| "{}".to_string());
                let escaped = escape_elixir(&json_str);
                setup_lines.push(format!("{name}_config = \"{escaped}\""));
                setup_lines.push(format!(
                    "{{:ok, {name}}} = {module_path}.{constructor_name}({name}_config)",
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        let val = if arg.field == "input" {
            Some(input)
        } else {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            input.get(field)
        };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Elixir functions have fixed positional arity — pass nil for optional args
                // rather than skipping them, so the call site has the correct arity.
                parts.push("nil".to_string());
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
                // For file_path args, prepend the path to the test_documents directory
                // relative to the e2e/elixir/ directory where `mix test` runs.
                if arg.arg_type == "file_path" {
                    if let Some(path_str) = v.as_str() {
                        let full_path = format!("{test_documents_path}/{path_str}");
                        parts.push(format!("\"{}\"", escape_elixir(&full_path)));
                        continue;
                    }
                }
                // For bytes args, use File.read! for file paths and Base.decode64! for base64.
                // Inline text (starts with '<', '{', '[' or contains spaces) is used as-is (UTF-8 binary).
                if arg.arg_type == "bytes" {
                    if let Some(raw) = v.as_str() {
                        let var_name = &arg.name;
                        if raw.starts_with('<') || raw.starts_with('{') || raw.starts_with('[') || raw.contains(' ') {
                            // Inline text — use as a binary string.
                            parts.push(format!("\"{}\"", escape_elixir(raw)));
                        } else {
                            let first = raw.chars().next().unwrap_or('\0');
                            let is_file_path = (first.is_ascii_alphanumeric() || first == '_')
                                && raw
                                    .find('/')
                                    .is_some_and(|slash_pos| slash_pos > 0 && raw[slash_pos + 1..].contains('.'));
                            if is_file_path {
                                // Looks like "dir/file.ext" — read from the
                                // configured test-documents directory.
                                let full_path = format!("{test_documents_path}/{raw}");
                                let escaped = escape_elixir(&full_path);
                                setup_lines.push(format!("{var_name} = File.read!(\"{escaped}\")"));
                                parts.push(var_name.to_string());
                            } else {
                                // Treat as base64-encoded binary.
                                setup_lines.push(format!(
                                    "{var_name} = Base.decode64!(\"{}\", padding: false)",
                                    escape_elixir(raw)
                                ));
                                parts.push(var_name.to_string());
                            }
                        }
                        continue;
                    }
                }
                // For json_object args with options_type+options_via, build a proper struct.
                if arg.arg_type == "json_object" && !v.is_null() {
                    if let (Some(_opts_type), Some(options_fn), Some(obj)) =
                        (options_type, options_default_fn, v.as_object())
                    {
                        // Add setup line to initialize options from default function.
                        let options_var = "options";
                        setup_lines.push(format!("{options_var} = {module_path}.{options_fn}()"));

                        // For each field in the options object, add a struct update line.
                        for (k, vv) in obj.iter() {
                            let snake_key = k.to_snake_case();
                            let elixir_val = if let Some(_enum_type) = enum_fields.get(k) {
                                if let Some(s) = vv.as_str() {
                                    let snake_val = s.to_snake_case();
                                    // Use atom for enum values, not string
                                    format!(":{snake_val}")
                                } else {
                                    json_to_elixir(vv)
                                }
                            } else {
                                json_to_elixir(vv)
                            };
                            setup_lines.push(format!(
                                "{options_var} = %{{{options_var} | {snake_key}: {elixir_val}}}"
                            ));
                        }

                        // Push the variable name as the argument.
                        parts.push(options_var.to_string());
                        continue;
                    }
                    // When element_type is set to a batch item type, wrap items with constructors.
                    if let Some(elem_type) = &arg.element_type {
                        if (elem_type == "BatchBytesItem" || elem_type == "BatchFileItem") && v.is_array() {
                            parts.push(emit_elixir_batch_item_array(v, elem_type));
                            continue;
                        }
                        // When element_type is set to a simple type (e.g. Vec<String>).
                        // The NIF accepts an Elixir list directly — emit one.
                        if v.is_array() {
                            parts.push(json_to_elixir(v));
                            continue;
                        }
                    }
                    // When there's no options_type+options_via, the Elixir NIF expects a JSON
                    // string (Option<String> decoded by serde_json) rather than an Elixir map.
                    // Serialize the JSON value to a string literal here.
                    if !v.is_null() {
                        let json_str = serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string());
                        let escaped = escape_elixir(&json_str);
                        parts.push(format!("\"{escaped}\""));
                        continue;
                    }
                }
                parts.push(json_to_elixir(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
}

/// Returns true if the field expression is a numeric/integer expression
/// (e.g., a `length(...)` call) rather than a string.
fn is_numeric_expr(field_expr: &str) -> bool {
    field_expr.starts_with("length(")
}

fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    module_path: &str,
    fields_enum: &std::collections::HashSet<String>,
    per_call_enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
) {
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct field accesses on the result.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content" => {
                let pred =
                    format!("Enum.all?({result_var}.chunks || [], fn c -> c.content != nil and c.content != \"\" end)");
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "      assert {pred}");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "      refute {pred}");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "      # skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            "chunks_have_embeddings" => {
                let pred = format!(
                    "Enum.all?({result_var}.chunks || [], fn c -> c.embedding != nil and c.embedding != [] end)"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "      assert {pred}");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "      refute {pred}");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "      # skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            // ---- EmbedResponse virtual fields ----
            // embed_texts returns [[float]] in Elixir — no wrapper struct.
            // result_var is the embedding matrix; use it directly.
            "embeddings" => {
                match assertion.assertion_type.as_str() {
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            let ex_val = json_to_elixir(val);
                            let _ = writeln!(out, "      assert length({result_var}) == {ex_val}");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let ex_val = json_to_elixir(val);
                            let _ = writeln!(out, "      assert length({result_var}) >= {ex_val}");
                        }
                    }
                    "not_empty" => {
                        let _ = writeln!(out, "      assert {result_var} != []");
                    }
                    "is_empty" => {
                        let _ = writeln!(out, "      assert {result_var} == []");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "      # skipped: unsupported assertion type on synthetic field 'embeddings'"
                        );
                    }
                }
                return;
            }
            "embedding_dimensions" => {
                let expr = format!("(if {result_var} == [], do: 0, else: length(hd({result_var})))");
                match assertion.assertion_type.as_str() {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let ex_val = json_to_elixir(val);
                            let _ = writeln!(out, "      assert {expr} == {ex_val}");
                        }
                    }
                    "greater_than" => {
                        if let Some(val) = &assertion.value {
                            let ex_val = json_to_elixir(val);
                            let _ = writeln!(out, "      assert {expr} > {ex_val}");
                        }
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "      # skipped: unsupported assertion type on synthetic field 'embedding_dimensions'"
                        );
                    }
                }
                return;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                let pred = match f.as_str() {
                    "embeddings_valid" => {
                        format!("Enum.all?({result_var}, fn e -> e != [] end)")
                    }
                    "embeddings_finite" => {
                        format!("Enum.all?({result_var}, fn e -> Enum.all?(e, fn v -> is_float(v) and v == v end) end)")
                    }
                    "embeddings_non_zero" => {
                        format!("Enum.all?({result_var}, fn e -> Enum.any?(e, fn v -> v != 0.0 end) end)")
                    }
                    "embeddings_normalized" => {
                        format!(
                            "Enum.all?({result_var}, fn e -> n = Enum.reduce(e, 0.0, fn v, acc -> acc + v * v end); abs(n - 1.0) < 1.0e-3 end)"
                        )
                    }
                    _ => unreachable!(),
                };
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "      assert {pred}");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "      refute {pred}");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "      # skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            // ---- keywords / keywords_count ----
            // Elixir ExtractionResult does not expose extracted_keywords; skip.
            "keywords" | "keywords_count" => {
                let _ = writeln!(
                    out,
                    "      # skipped: field '{f}' not available on Elixir ExtractionResult"
                );
                return;
            }
            _ => {}
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    // When `result_is_simple`, the bound result is the value itself (e.g. a binary)
    // so `is_valid_for_result` is meaningless — fall through and emit the assertion
    // against the bare result_var below.
    if !result_is_simple {
        if let Some(f) = &assertion.field {
            if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
                let _ = writeln!(out, "      # skipped: field '{f}' not available on result type");
                return;
            }
        }
    }

    // result_is_simple: when the call returns the value itself (e.g. a binary for
    // `speech` / `file_content`), bypass the field accessor and assert against the
    // bound `result` variable directly.
    let field_expr = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "elixir", result_var),
            _ => result_var.to_string(),
        }
    };

    // Only wrap in String.trim/0 when the expression is actually a string.
    // Numeric expressions (e.g., length(...)) must not be wrapped.
    let is_numeric = is_numeric_expr(&field_expr);
    // Detect whether the field resolves to an enum type. Rustler binds Rust
    // enums as atoms (e.g. `:stop`), so calling `String.trim/1` on them raises
    // FunctionClauseError. Coerce via `to_string/1` before string ops. Look up
    // both the global `[crates.e2e] fields_enum` set AND the per-call override
    // `[crates.e2e.calls.<x>.overrides.elixir] enum_fields = { ... }` so a single
    // config entry already populated for the C#/Java/Python sides applies here.
    let field_is_enum = assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        fields_enum.contains(f)
            || fields_enum.contains(resolved)
            || per_call_enum_fields.contains_key(f)
            || per_call_enum_fields.contains_key(resolved)
    });
    let coerced_field_expr = if field_is_enum {
        format!("to_string({field_expr})")
    } else {
        field_expr.clone()
    };
    let trimmed_field_expr = if is_numeric {
        field_expr.clone()
    } else {
        format!("String.trim({coerced_field_expr})")
    };

    // Detect whether the assertion field resolves to an array type so that
    // contains assertions can iterate items instead of calling to_string on the list.
    let field_is_array = assertion
        .field
        .as_deref()
        .filter(|f| !f.is_empty())
        .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let elixir_val = json_to_elixir(expected);
                // Apply String.trim only for string comparisons, not numeric ones.
                let is_string_expected = expected.is_string();
                if is_string_expected && !is_numeric {
                    let _ = writeln!(out, "      assert {trimmed_field_expr} == {elixir_val}");
                } else if field_is_enum {
                    let _ = writeln!(out, "      assert {coerced_field_expr} == {elixir_val}");
                } else {
                    let _ = writeln!(out, "      assert {field_expr} == {elixir_val}");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let elixir_val = json_to_elixir(expected);
                if field_is_array && expected.is_string() {
                    // List of structs: check if any item's text representation contains the value.
                    let _ = writeln!(
                        out,
                        "      assert Enum.any?({field_expr}, fn item -> Enum.any?(alef_e2e_item_texts(item), &String.contains?(&1, {elixir_val})) end)"
                    );
                } else {
                    // Use to_string() to handle atoms (enums) as well as strings
                    let _ = writeln!(
                        out,
                        "      assert String.contains?(to_string({field_expr}), {elixir_val})"
                    );
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let elixir_val = json_to_elixir(val);
                    if field_is_array && val.is_string() {
                        let _ = writeln!(
                            out,
                            "      assert Enum.any?({field_expr}, fn item -> Enum.any?(alef_e2e_item_texts(item), &String.contains?(&1, {elixir_val})) end)"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "      assert String.contains?(to_string({field_expr}), {elixir_val})"
                        );
                    }
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let elixir_val = json_to_elixir(expected);
                if field_is_array && expected.is_string() {
                    let _ = writeln!(
                        out,
                        "      refute Enum.any?({field_expr}, fn item -> Enum.any?(alef_e2e_item_texts(item), &String.contains?(&1, {elixir_val})) end)"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "      refute String.contains?(to_string({field_expr}), {elixir_val})"
                    );
                }
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "      assert {field_expr} != \"\"");
        }
        "is_empty" => {
            if is_numeric {
                // length(...) == 0
                let _ = writeln!(out, "      assert {field_expr} == 0");
            } else {
                // Handle nil (None) as empty
                let _ = writeln!(out, "      assert is_nil({field_expr}) or {trimmed_field_expr} == \"\"");
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(json_to_elixir).collect();
                let list_str = items.join(", ");
                let _ = writeln!(
                    out,
                    "      assert Enum.any?([{list_str}], fn v -> String.contains?(to_string({field_expr}), v) end)"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let elixir_val = json_to_elixir(val);
                let _ = writeln!(out, "      assert {field_expr} > {elixir_val}");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let elixir_val = json_to_elixir(val);
                let _ = writeln!(out, "      assert {field_expr} < {elixir_val}");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let elixir_val = json_to_elixir(val);
                let _ = writeln!(out, "      assert {field_expr} >= {elixir_val}");
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let elixir_val = json_to_elixir(val);
                let _ = writeln!(out, "      assert {field_expr} <= {elixir_val}");
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let elixir_val = json_to_elixir(expected);
                let _ = writeln!(out, "      assert String.starts_with?({field_expr}, {elixir_val})");
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let elixir_val = json_to_elixir(expected);
                let _ = writeln!(out, "      assert String.ends_with?({field_expr}, {elixir_val})");
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "      assert String.length({field_expr}) >= {n}");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "      assert String.length({field_expr}) <= {n}");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "      assert length({field_expr}) >= {n}");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "      assert length({field_expr}) == {n}");
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "      assert {field_expr} == true");
        }
        "is_false" => {
            let _ = writeln!(out, "      assert {field_expr} == false");
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                let call_expr = build_elixir_method_call(result_var, method_name, assertion.args.as_ref(), module_path);
                let check = assertion.check.as_deref().unwrap_or("is_true");
                match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let elixir_val = json_to_elixir(val);
                            let _ = writeln!(out, "      assert {call_expr} == {elixir_val}");
                        }
                    }
                    "is_true" => {
                        let _ = writeln!(out, "      assert {call_expr} == true");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "      assert {call_expr} == false");
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(out, "      assert {call_expr} >= {n}");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(out, "      assert length({call_expr}) >= {n}");
                        }
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            let elixir_val = json_to_elixir(val);
                            let _ = writeln!(out, "      assert String.contains?({call_expr}, {elixir_val})");
                        }
                    }
                    "is_error" => {
                        let _ = writeln!(out, "      assert_raise RuntimeError, fn -> {call_expr} end");
                    }
                    other_check => {
                        panic!("Elixir e2e generator: unsupported method_result check type: {other_check}");
                    }
                }
            } else {
                panic!("Elixir e2e generator: method_result assertion missing 'method' field");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let elixir_val = json_to_elixir(expected);
                let _ = writeln!(out, "      assert Regex.match?(~r/{elixir_val}/, {field_expr})");
            }
        }
        "not_error" => {
            // Already handled — the call would fail if it returned {:error, _}.
        }
        "error" => {
            // Handled at the test level.
        }
        other => {
            panic!("Elixir e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Build an Elixir call expression for a `method_result` assertion on a tree-sitter result.
/// Maps method names to the appropriate `module_path` function calls.
fn build_elixir_method_call(
    result_var: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
    module_path: &str,
) -> String {
    match method_name {
        "root_child_count" => format!("{module_path}.root_child_count({result_var})"),
        "has_error_nodes" => format!("{module_path}.tree_has_error_nodes({result_var})"),
        "error_count" | "tree_error_count" => format!("{module_path}.tree_error_count({result_var})"),
        "tree_to_sexp" => format!("{module_path}.tree_to_sexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{module_path}.tree_contains_node_type({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{module_path}.find_nodes_by_type({result_var}, \"{node_type}\")")
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
            format!("{module_path}.run_query({result_var}, \"{language}\", \"{query_source}\", source)")
        }
        _ => format!("{module_path}.{method_name}({result_var})"),
    }
}

/// Convert a category name to an Elixir module-safe PascalCase name.
fn elixir_module_name(category: &str) -> String {
    use heck::ToUpperCamelCase;
    category.to_upper_camel_case()
}

/// Convert a `serde_json::Value` to an Elixir literal string.
fn json_to_elixir(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_elixir(s)),
        serde_json::Value::Bool(true) => "true".to_string(),
        serde_json::Value::Bool(false) => "false".to_string(),
        serde_json::Value::Number(n) => {
            // Elixir requires floats to have a decimal point and does not accept
            // `e+N` exponent notation. Strip the `+` and ensure there is a decimal
            // point before any `e` exponent marker (e.g. `1e-10` → `1.0e-10`).
            let s = n.to_string().replace("e+", "e");
            if s.contains('e') && !s.contains('.') {
                // Insert `.0` before the `e` so Elixir treats this as a float.
                s.replacen('e', ".0e", 1)
            } else {
                s
            }
        }
        serde_json::Value::Null => "nil".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_elixir).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("\"{}\" => {}", escape_elixir(k), json_to_elixir(v)))
                .collect();
            format!("%{{{}}}", entries.join(", "))
        }
    }
}

/// Build an Elixir visitor map and add setup line. Returns the visitor variable name.
fn build_elixir_visitor(setup_lines: &mut Vec<String>, visitor_spec: &crate::fixture::VisitorSpec) -> String {
    use std::fmt::Write as FmtWrite;
    let mut visitor_obj = String::new();
    let _ = writeln!(visitor_obj, "%{{");
    for (method_name, action) in &visitor_spec.callbacks {
        emit_elixir_visitor_method(&mut visitor_obj, method_name, action);
    }
    let _ = writeln!(visitor_obj, "    }}");

    setup_lines.push(format!("visitor = {visitor_obj}"));
    "visitor".to_string()
}

/// Emit an Elixir visitor method for a callback action.
fn emit_elixir_visitor_method(out: &mut String, method_name: &str, action: &CallbackAction) {
    use std::fmt::Write as FmtWrite;

    // Elixir uses atom keys and handle_ prefix
    let handle_method = format!("handle_{}", &method_name[6..]); // strip "visit_" prefix
    // The Rust NIF bridge packages every visitor argument (`_ctx`, `_text`, …) into a
    // single map and invokes the user's anonymous function with that map. Generating
    // multi-arity functions like `fn(_ctx, _text) ->` therefore raised BadArityError
    // ("arity 2 called with 1 argument") at runtime. Generate arity-1 functions that
    // accept the args map (and ignore it) to match the bridge's calling convention.

    // CustomTemplate needs to read from args; other actions can ignore it.
    let arg_binding = match action {
        CallbackAction::CustomTemplate { .. } => "args",
        _ => "_args",
    };
    let _ = writeln!(out, "      :{handle_method} => fn({arg_binding}) ->");
    match action {
        CallbackAction::Skip => {
            let _ = writeln!(out, "        :skip");
        }
        CallbackAction::Continue => {
            let _ = writeln!(out, "        :continue");
        }
        CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "        :preserve_html");
        }
        CallbackAction::Custom { output } => {
            let escaped = escape_elixir(output);
            let _ = writeln!(out, "        {{:custom, \"{escaped}\"}}");
        }
        CallbackAction::CustomTemplate { template } => {
            // Build a <> concatenation expression so {key} placeholders are substituted
            // from the args map at runtime without embedding double-quoted strings inside
            // a double-quoted string literal.
            let expr = template_to_elixir_concat(template);
            let _ = writeln!(out, "        {{:custom, {expr}}}");
        }
    }
    let _ = writeln!(out, "      end,");
}

/// Convert a template like `"_{text}_"` into an Elixir `<>` concat expression:
/// `"_" <> Map.get(args, "text", "") <> "_"`.
/// Static parts are escaped via `escape_elixir`; `{key}` placeholders become
/// `Map.get(args, "key", "")` lookups into the JSON-decoded args map.
fn template_to_elixir_concat(template: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut static_buf = String::new();
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' {
            let mut key = String::new();
            let mut closed = false;
            for kc in chars.by_ref() {
                if kc == '}' {
                    closed = true;
                    break;
                }
                key.push(kc);
            }
            if closed && !key.is_empty() {
                if !static_buf.is_empty() {
                    let escaped = escape_elixir(&static_buf);
                    parts.push(format!("\"{escaped}\""));
                    static_buf.clear();
                }
                let escaped_key = escape_elixir(&key);
                parts.push(format!("Map.get(args, \"{escaped_key}\", \"\")"));
            } else {
                static_buf.push('{');
                static_buf.push_str(&key);
                if !closed {
                    // unclosed brace — treat remaining as literal
                }
            }
        } else {
            static_buf.push(ch);
        }
    }

    if !static_buf.is_empty() {
        let escaped = escape_elixir(&static_buf);
        parts.push(format!("\"{escaped}\""));
    }

    if parts.is_empty() {
        return "\"\"".to_string();
    }
    parts.join(" <> ")
}

fn fixture_has_elixir_callable(fixture: &Fixture, e2e_config: &E2eConfig) -> bool {
    // HTTP fixtures are handled separately — not our concern here.
    if fixture.is_http_test() {
        return false;
    }
    let call_config = e2e_config.resolve_call_for_fixture(fixture.call.as_deref(), &fixture.input);
    let elixir_override = call_config
        .overrides
        .get("elixir")
        .or_else(|| e2e_config.call.overrides.get("elixir"));
    // When a client_factory is configured the fixture is callable via the client pattern.
    if elixir_override.and_then(|o| o.client_factory.as_deref()).is_some() {
        return true;
    }
    // Elixir bindings expose functions via module-level callables.
    // Like Python and Node, Elixir can call the base function directly without requiring
    // a language-specific override. The function can come from either the override or
    // the default [e2e.call] configuration.
    let function_from_override = elixir_override.and_then(|o| o.function.as_deref());

    // If there's an override function, use it. Otherwise, Elixir can use the base function.
    function_from_override.is_some() || !call_config.function.is_empty()
}
