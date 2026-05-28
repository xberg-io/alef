//! Elixir e2e test generator using ExUnit.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{escape_elixir, sanitize_filename, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, CallbackAction, Fixture, FixtureGroup, HttpFixture, ValidationErrorExpectation};
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
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
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
        // (e.g., "sample_markdown" -> "SampleMarkdown").
        // If the override already contains "." (e.g., "Elixir.SampleMarkdown"), use as-is.
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
                let cc = e2e_config.resolve_call_for_fixture(
                    f.call.as_deref(),
                    &f.id,
                    &f.resolved_category(),
                    &f.tags,
                    &f.input,
                );
                let elixir_override = cc
                    .overrides
                    .get("elixir")
                    .or_else(|| e2e_config.call.overrides.get("elixir"));
                elixir_override.and_then(|o| o.client_factory.as_deref()).is_some()
            })
        });

        // Resolve package reference (path or version) for the NIF dependency.
        let pkg_ref = e2e_config.resolve_package(lang);
        let pkg_dep_ref = if has_nif_tests {
            match e2e_config.dep_mode {
                crate::e2e::config::DependencyMode::Local => pkg_ref
                    .as_ref()
                    .and_then(|p| p.path.as_deref())
                    .unwrap_or("../../packages/elixir")
                    .to_string(),
                crate::e2e::config::DependencyMode::Registry => pkg_ref
                    .as_ref()
                    .and_then(|p| p.version.clone())
                    .or_else(|| config.resolved_version())
                    .unwrap_or_else(|| "0.1.0".to_string()),
            }
        } else {
            String::new()
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
            content: render_mix_exs(
                &pkg_atom,
                &pkg_dep_ref,
                e2e_config.dep_mode,
                has_http_tests,
                has_nif_tests,
            ),
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
            let content = render_test_file(
                &group.category,
                &active,
                e2e_config,
                &module_path,
                &function_name,
                result_var,
                &e2e_config.call.args,
                options_type.as_deref(),
                options_default_fn.as_deref(),
                enum_fields,
                handle_struct_type.as_deref(),
                handle_atom_list_fields,
                &config.adapters,
                enums,
                config,
                type_defs,
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
# If MOCK_SERVER_URL is already set, a parent process (e.g. `alef test-apps
# run`) started a shared mock-server and exported its URL (plus any
# MOCK_SERVERS / MOCK_SERVER_<FIXTURE_ID> vars). Use it as-is and do NOT
# spawn our own server.
mock_server_bin = Path.expand("../../rust/target/release/mock-server", __DIR__)
fixtures_dir = Path.expand("../../../fixtures", __DIR__)

unless System.get_env("MOCK_SERVER_URL") do
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
    dep_mode: crate::e2e::config::DependencyMode,
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
            crate::e2e::config::DependencyMode::Local => {
                format!("      {{:{pkg_atom}, path: \"{pkg_path}\"}}")
            }
            crate::e2e::config::DependencyMode::Registry => {
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
    args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
    options_default_fn: Option<&str>,
    enum_fields: &HashMap<String, String>,
    handle_struct_type: Option<&str>,
    handle_atom_list_fields: &std::collections::HashSet<String>,
    adapters: &[crate::core::config::extras::AdapterConfig],
    enums: &[crate::core::ir::EnumDef],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
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
        let cc = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        let fr = FieldResolver::new(
            e2e_config.effective_fields(cc),
            e2e_config.effective_fields_optional(cc),
            e2e_config.effective_result_fields(cc),
            e2e_config.effective_fields_array(cc),
            &std::collections::HashSet::new(),
        );
        fixture.assertions.iter().any(|a| {
            matches!(a.assertion_type.as_str(), "contains" | "contains_all" | "not_contains")
                && a.field
                    .as_deref()
                    .is_some_and(|f| !f.is_empty() && fr.is_array(fr.resolve(f)))
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
        let _ = writeln!(out, "        str -> [inspect(str)]");
        let _ = writeln!(out, "      end");
        let _ = writeln!(out, "    end)");
        let _ = writeln!(out, "  end");
    }

    // Emit a helper to convert FormatMetadata struct to a string representation
    // (pattern-match on the image field and extract the format string).
    let has_format_metadata = fixtures.iter().any(|fixture| {
        fixture.assertions.iter().any(|a| {
            a.field
                .as_deref()
                .is_some_and(|f| f.contains("format") && f.contains("metadata"))
        })
    });
    if has_format_metadata {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "  defp alef_e2e_format_to_string(value) when is_binary(value), do: value"
        );
        let _ = writeln!(out, "  defp alef_e2e_format_to_string(metadata) do");
        let _ = writeln!(out, "    case metadata.image do");
        let _ = writeln!(out, "      %{{format: fmt}} when is_binary(fmt) -> fmt");
        let _ = writeln!(out, "      _ ->");
        let _ = writeln!(out, "        case metadata.pdf do");
        let _ = writeln!(out, "          %{{}} -> \"PDF\"");
        let _ = writeln!(out, "          _ ->");
        let _ = writeln!(out, "            case metadata.html do");
        let _ = writeln!(out, "              %{{}} -> \"HTML\"");
        let _ = writeln!(out, "              _ -> inspect(metadata)");
        let _ = writeln!(out, "            end");
        let _ = writeln!(out, "        end");
        let _ = writeln!(out, "    end");
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
                options_type,
                options_default_fn,
                enum_fields,
                handle_struct_type,
                handle_atom_list_fields,
                adapters,
                enums,
                config,
                type_defs,
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

/// Req option that forces the HTTP/1 protocol on the underlying Finch/Mint
/// connection. The shared e2e mock server is a plain HTTP/1.1 server, so Req's
/// default HTTP/2 negotiation fails with `:pool_not_available` (no HTTP/2 Finch
/// pool is started). Pinning the connection to HTTP/1 makes every request use
/// the available HTTP/1 pool.
const REQ_HTTP1_OPT: &str = "connect_options: [protocols: [:http1]]";

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
        // Force HTTP/1 on every request: the mock server is HTTP/1.1 and Req's
        // default HTTP/2 negotiation fails with `:pool_not_available`.
        let mut opts: Vec<String> = vec![REQ_HTTP1_OPT.to_string()];

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
            // `opts` always carries at least the HTTP/1 protocol option.
            let opts_str = opts.join(", ");
            let _ = writeln!(
                out,
                "      {{:ok, response}} = Req.{method}(url: {url_expr}, {opts_str})"
            );
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
    _default_module_path: &str,
    _default_function_name: &str,
    _default_result_var: &str,
    _args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
    options_default_fn: Option<&str>,
    _enum_fields: &HashMap<String, String>,
    handle_struct_type: Option<&str>,
    _handle_atom_list_fields: &std::collections::HashSet<String>,
    adapters: &[crate::core::config::extras::AdapterConfig],
    enums: &[crate::core::ir::EnumDef],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) {
    let test_name = sanitize_ident(&fixture.id);
    let test_label = fixture.id.replace('"', "\\\"");

    // Helper function to extract module-level definitions from a setup_block that may
    // contain a trait-bridge marker. Trait-bridge setup blocks are formatted as:
    //   <module definitions ending with "end\n">
    //   \n__TRAIT_BRIDGE_MODULE_DEFS_END__\n
    //   <test-function-level setup>
    // We split on the marker and emit module defs before the test, then use only the setup part.
    fn extract_trait_bridge_parts(setup_block: &str) -> (String, String) {
        if let Some(pos) = setup_block.find("__TRAIT_BRIDGE_MODULE_DEFS_END__") {
            // Find the start and end of the marker line
            let marker_start = setup_block[..pos].rfind('\n').unwrap_or(0);
            let marker_end = if let Some(nl) = setup_block[pos + 32..].find('\n') {
                pos + 32 + nl + 1
            } else {
                setup_block.len()
            };
            let module_defs = setup_block[..marker_start].trim_end().to_string();
            let test_setup = setup_block[marker_end..].trim_start().to_string();
            (module_defs, test_setup)
        } else {
            // No marker: entire block is test-level setup (legacy or non-trait-bridge code)
            (String::new(), setup_block.to_string())
        }
    }

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
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Build per-call field resolver using the effective field sets for this call.
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    );
    let field_resolver = &call_field_resolver;
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

    // Compute module_path and function_name from the resolved call config.
    // call_config is resolved via resolve_call_for_fixture which applies select_when auto-routing,
    // so we always use it — whether or not fixture.call was explicitly set.
    // Apply Elixir-specific PascalCase conversion.
    let raw_module = call_overrides
        .and_then(|o| o.module.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.module.clone());
    let module_path = if raw_module.contains('.') || raw_module.chars().next().is_some_and(|c| c.is_uppercase()) {
        raw_module
    } else {
        elixir_module_name(&raw_module)
    };
    let function_name = if call_config.r#async && !base_fn.ends_with("_async") && !base_fn.ends_with("_stream") {
        format!("{base_fn}_async")
    } else {
        base_fn
    };
    let result_var = call_config.result_var.clone();

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    // Validation-category fixtures expect engine creation itself to fail (bad config).
    // Other expects_error fixtures (e.g. error_*) construct a valid engine and expect the
    // *operation under test* to fail. We need different shapes for these two cases.
    let validation_creation_failure = expects_error && fixture.resolved_category() == "validation";

    // Use args and options from the resolved call_config (which may have been auto-routed via select_when),
    // falling back to the fixture-level defaults if not available.
    let co = call_config.overrides.get(lang);
    let empty_enum_fields_local: HashMap<String, String> = HashMap::new();
    let empty_atom_fields_local: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Use the call config's args, not the fallback global args.
    // This ensures that functions like list_document_extractors with args=[] stay empty,
    // instead of falling back to the global [crates.e2e.call] args which are meant for extract_file.
    let resolved_args = fixture.resolved_args(call_config);
    let resolved_options_type = co
        .and_then(|o| o.options_type.clone())
        .or_else(|| options_type.map(|s| s.to_string()));
    let resolved_options_default_fn = co
        .and_then(|o| o.options_via.clone())
        .or_else(|| options_default_fn.map(|s| s.to_string()));
    let resolved_enum_fields_ref = co.map(|o| &o.enum_fields).unwrap_or(&empty_enum_fields_local);
    let resolved_handle_struct_type = co
        .and_then(|o| o.handle_struct_type.clone())
        .or_else(|| handle_struct_type.map(|s| s.to_string()));
    let resolved_handle_atom_list_fields_ref = co
        .map(|o| &o.handle_atom_list_fields)
        .unwrap_or(&empty_atom_fields_local);

    let test_documents_path = e2e_config.test_documents_relative_from(0);
    let adapter_request_type: Option<String> = adapters
        .iter()
        .find(|a| a.name == call_config.function.as_str())
        .and_then(|a| a.request_type.as_deref())
        .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());
    let (mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        resolved_args,
        &module_path,
        resolved_options_type.as_deref(),
        resolved_options_default_fn.as_deref(),
        resolved_enum_fields_ref,
        fixture,
        resolved_handle_struct_type.as_deref(),
        resolved_handle_atom_list_fields_ref,
        &test_documents_path,
        adapter_request_type.as_deref(),
        enums,
        config,
        type_defs,
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

    // Extract trait-bridge module definitions from setup_lines so they can be emitted
    // at module level (before the describe block), not indented inside the test function.
    // Trait-bridge setup blocks are formatted with a marker: module defs, then marker, then test setup.
    let mut trait_bridge_module_defs = Vec::new();
    let mut cleaned_setup_lines = Vec::new();
    for line in setup_lines.iter() {
        if line.contains("__TRAIT_BRIDGE_MODULE_DEFS_END__") {
            // Split this line on the marker
            let (module_part, test_part) = extract_trait_bridge_parts(line);
            // Emit module defs at module level (no indentation)
            for module_line in module_part.lines() {
                if !module_line.is_empty() {
                    trait_bridge_module_defs.push(module_line.to_string());
                }
            }
            // Emit test-level part indented in the test function
            for test_line in test_part.lines() {
                if !test_line.is_empty() {
                    cleaned_setup_lines.push(test_line.to_string());
                }
            }
        } else {
            cleaned_setup_lines.push(line.clone());
        }
    }

    // Emit trait-bridge module definitions at module level (before describe block)
    for module_def_line in &trait_bridge_module_defs {
        let _ = writeln!(out, "{module_def_line}");
    }

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
        for line in &cleaned_setup_lines {
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
            let call_invocation = if effective_args.is_empty() {
                format!("{module_path}.{function_name}()")
            } else {
                format!("{module_path}.{function_name}({effective_args})")
            };
            let _ = writeln!(out, "      assert {{:error, _}} = {call_invocation}");
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
        for line in &cleaned_setup_lines {
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
                "      {{:ok, client}} = {module_path}.{factory}(\"test-key\", base_url: {base_url_expr})"
            );
        }
        let call_invocation = if effective_args.is_empty() {
            format!("{module_path}.{function_name}()")
        } else {
            format!("{module_path}.{function_name}({effective_args})")
        };
        let _ = writeln!(out, "      assert {{:error, _}} = {call_invocation}");
        if needs_api_key_skip {
            let _ = writeln!(out, "      end");
        }
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "  end");
        return;
    }

    for line in &cleaned_setup_lines {
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
                "      {{api_key_val, client_opts}} = if api_key_val && api_key_val != \"\" do"
            );
            let _ = writeln!(
                out,
                "        IO.puts(\"{fixture_id}: using real API ({api_key_var} is set)\")"
            );
            let _ = writeln!(out, "        {{api_key_val, []}}");
            let _ = writeln!(out, "      else");
            let _ = writeln!(
                out,
                "        IO.puts(\"{fixture_id}: using mock server ({api_key_var} not set)\")"
            );
            let _ = writeln!(out, "        {{\"test-key\", [base_url: {mock_url_expr}]}}");
            let _ = writeln!(out, "      end");
            let _ = writeln!(
                out,
                "      {{:ok, client}} = {module_path}.{factory}(api_key_val, client_opts)"
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
                "      {{:ok, client}} = {module_path}.{factory}(\"test-key\", base_url: {base_url_expr})"
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

    // Streaming detection (call-level `streaming` opt-out is honored).
    let is_streaming = crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming);
    // For streaming fixtures the stream is bound to `result_var` first, then drained into `chunks`.
    let chunks_var = "chunks";

    // If the result variable is never referenced in assertions or streaming operations,
    // prefix it with _ to avoid "unused variable" warnings in mix compile --warnings-as-errors.
    let actual_result_var = if fixture.assertions.is_empty() && !is_streaming {
        format!("_{result_var}")
    } else {
        result_var.to_string()
    };

    // Render function call: omit args entirely if effective_args is empty (no-arg functions).
    // This prevents emitting `func(nil)` which causes FunctionClauseError on nil-free function signatures.
    let call_invocation = if effective_args.is_empty() {
        format!("{module_path}.{function_name}()")
    } else {
        format!("{module_path}.{function_name}({effective_args})")
    };

    if returns_result {
        let _ = writeln!(out, "      {{:ok, {actual_result_var}}} = {call_invocation}");
    } else {
        // Non-Result function returns value directly (e.g., bool, String).
        let _ = writeln!(out, "      {actual_result_var} = {call_invocation}");
    }

    // For streaming fixtures, drain the stream into a list before asserting.
    if is_streaming {
        if let Some(collect) = crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet(
            "elixir",
            &result_var,
            chunks_var,
        ) {
            let _ = writeln!(out, "      {collect}");
        }
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            if is_streaming { chunks_var } else { &result_var },
            field_resolver,
            &module_path,
            e2e_config.effective_fields_enum(call_config),
            resolved_enum_fields_ref,
            result_is_simple,
            is_streaming,
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
    args: &[crate::e2e::config::ArgMapping],
    module_path: &str,
    options_type: Option<&str>,
    options_default_fn: Option<&str>,
    enum_fields: &HashMap<String, String>,
    fixture: &crate::e2e::fixture::Fixture,
    _handle_struct_type: Option<&str>,
    _handle_atom_list_fields: &std::collections::HashSet<String>,
    test_documents_path: &str,
    adapter_request_type: Option<&str>,
    enums: &[crate::core::ir::EnumDef],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> (Vec<String>, String) {
    let fixture_id = &fixture.id;
    if args.is_empty() {
        // No args config: pass the whole input only when it's non-empty AND not just the harness setup dict.
        // Functions with no parameters (e.g. language_count) have empty input
        // and must be called with no arguments — not with `%{}`.
        // Filter out the harness' internal "setup" field — it's not part of the fixture's actual input.
        let cleaned_input = match input {
            serde_json::Value::Object(m) => {
                let mut cleaned = m.clone();
                cleaned.remove("setup");
                if cleaned.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::Object(cleaned)
                }
            }
            other => other.clone(),
        };
        let is_empty_input = matches!(cleaned_input, serde_json::Value::Null);
        if is_empty_input {
            return (Vec::new(), String::new());
        }
        return (Vec::new(), json_to_elixir(&cleaned_input));
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    // NOTE: Elixir requires all positional args before keyword args. To avoid syntax errors,
    // count how many optional args will be rendered as keywords upfront, then decide
    // whether json_object args should be positional or keyword. This aligns with the
    // Rustler backend's keyword-opts threshold: use keyword form for 2+ trailing optional
    // params, stay positional for 1 or 0.
    let trailing_keyword_count = args
        .iter()
        .rev()
        .take_while(|a| a.optional)
        .filter(|a| {
            // An arg will be rendered as keyword if it's optional AND has a provided value
            // that's not null. We can't fully evaluate this without checking the input,
            // but we can count optional params at the end — a conservative heuristic.
            a.arg_type != "mock_url" && a.arg_type != "mock_url_list" && a.arg_type != "handle"
        })
        .count();
    let use_keyword_form_for_optional_args = trailing_keyword_count >= 2;

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
            if let Some(req_type) = adapter_request_type {
                let req_var = format!("{}_req", arg.name);
                setup_lines.push(format!("{req_var} = %{module_path}.{req_type}{{url: {}}}", arg.name,));
                parts.push(req_var);
            } else {
                parts.push(arg.name.clone());
            }
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            // list of URLs: each element is either a bare path (`/seed1`) — prefixed
            // with the per-fixture mock-server URL at runtime — or an absolute URL
            // kept as-is. Mirrors `mock_url` resolution: `MOCK_SERVER_<FIXTURE_ID>`
            // first, then `MOCK_SERVER_URL/fixtures/<id>`. Without this branch the
            // codegen falls back to a JSON-array literal of bare relative paths and
            // the Rust HTTP client rejects them.
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field).unwrap_or(&serde_json::Value::Null);
            let paths: Vec<String> = if let Some(arr) = val.as_array() {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", escape_elixir(s))))
                    .collect()
            } else {
                Vec::new()
            };
            let paths_literal = paths.join(", ");
            let name = &arg.name;
            setup_lines.push(format!(
                "{name}_base = System.get_env(\"{env_key}\") || ((System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\")"
            ));
            setup_lines.push(format!(
                "{name} = Enum.map([{paths_literal}], fn p -> if String.starts_with?(p, \"http\"), do: p, else: {name}_base <> p end)"
            ));
            parts.push(name.clone());
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

        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    // Collect methods from both the main trait and its super-trait (if present).
                    // The super-trait methods are needed so stubs implement the full interface.
                    let mut methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| t.methods.iter().collect())
                        .unwrap_or_default();

                    // If there's a super-trait, also collect its methods.
                    if let Some(super_trait) = &trait_bridge.super_trait {
                        if let Some(super_type) = type_defs.iter().find(|t| &t.name == super_trait) {
                            for method in &super_type.methods {
                                // Only add if not already present (avoid duplicates).
                                if !methods.iter().any(|m| m.name == method.name) {
                                    methods.push(method);
                                }
                            }
                        }
                    }

                    // Derive the NIF module from the test module path: the NIF module
                    // follows the "{AppModule}.Native" convention used by the Elixir scaffold.
                    let elixir_nif_module = format!("{module_path}.Native");
                    let emission = emit_test_backend(trait_bridge, &methods, fixture, &elixir_nif_module);
                    setup_lines.push(emission.setup_block);
                    parts.push(emission.arg_expr);

                    // For register_fn traits (plugin pattern), Rustler requires a second "name" argument.
                    // Extract the backend name from fixture input (same logic as emit_test_backend).
                    if trait_bridge.register_fn.is_some() {
                        let backend_name = extract_backend_name_from_input(&fixture.input, &fixture.id);
                        parts.push(format!("\"{}\"", escape_elixir(&backend_name)));
                    }
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("elixir");
            setup_lines.push(format!("# {}", emission.arg_expr));
            parts.push("nil".to_string());
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
                // Optional params map to the keyword-opts `opts \\ []` argument.
                // When the value is absent, omit the keyword entirely — the default `[]` applies.
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
                        let formatted = format!("\"{}\"", escape_elixir(&full_path));
                        if arg.optional {
                            parts.push(format!("{}: {formatted}", arg.name));
                        } else {
                            parts.push(formatted);
                        }
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
                            let formatted = format!("\"{}\"", escape_elixir(raw));
                            if arg.optional {
                                parts.push(format!("{}: {formatted}", arg.name));
                            } else {
                                parts.push(formatted);
                            }
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
                                if arg.optional {
                                    parts.push(format!("{}: {var_name}", arg.name));
                                } else {
                                    parts.push(var_name.to_string());
                                }
                            } else {
                                // Treat as base64-encoded binary.
                                setup_lines.push(format!(
                                    "{var_name} = Base.decode64!(\"{}\", padding: false)",
                                    escape_elixir(raw)
                                ));
                                if arg.optional {
                                    parts.push(format!("{}: {var_name}", arg.name));
                                } else {
                                    parts.push(var_name.to_string());
                                }
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
                        // Optional args (with `\\ []` or `\\ nil`) always use keyword form
                        // so that the facade can handle them via Keyword.get() or defaults.
                        parts.push(format!("{}: {options_var}", arg.name));
                        continue;
                    }
                    // When options_type is set but options_via is NOT, emit a struct-literal form.
                    // The auto-generated Rustler facade signature (`def f(html, options \\ nil)
                    // when is_map(options)`) requires a map, not a JSON string — and Elixir
                    // structs ARE maps, so a struct literal matches the guard. Falling through
                    // to the JSON-string emission below would yield `f(html, "{json}")`, which
                    // crashes the facade with FunctionClauseError. Emit positional/keyword
                    // form per `use_keyword_form_for_optional_args` to mirror the threshold
                    // applied to JSON-string emission.
                    if let (Some(opts_type), None, Some(obj)) = (options_type, options_default_fn, v.as_object()) {
                        let options_var = "options";
                        let mut field_strs = Vec::new();
                        for (k, vv) in obj.iter() {
                            let snake_key = k.to_snake_case();
                            let elixir_val = if enum_fields.contains_key(k) {
                                if let Some(s) = vv.as_str() {
                                    let snake_val = s.to_snake_case();
                                    format!(":{snake_val}")
                                } else {
                                    json_to_elixir(vv)
                                }
                            } else {
                                json_to_elixir(vv)
                            };
                            field_strs.push(format!("{snake_key}: {elixir_val}"));
                        }
                        let fields = field_strs.join(", ");
                        setup_lines.push(format!("{options_var} = %{module_path}.{opts_type}{{{fields}}}"));
                        if use_keyword_form_for_optional_args && arg.optional {
                            parts.push(format!("{}: {options_var}", arg.name));
                        } else {
                            parts.push(options_var.to_string());
                        }
                        continue;
                    }
                    // When element_type is set to a batch item type, wrap items with constructors.
                    if let Some(elem_type) = &arg.element_type {
                        if (elem_type == "BatchBytesItem" || elem_type == "BatchFileItem") && v.is_array() {
                            let formatted = emit_elixir_batch_item_array(v, elem_type);
                            if arg.optional {
                                parts.push(format!("{}: {formatted}", arg.name));
                            } else {
                                parts.push(formatted);
                            }
                            continue;
                        }
                        // Internally-tagged enums (#[serde(tag = "type")]) — emit a list of
                        // Rustler NifTaggedEnum tuples. `:variant_atom` for unit variants,
                        // `{:variant_atom, %{field: value}}` for struct variants. Variant
                        // and field atoms are derived from Rust names via snake_case;
                        // Rustler's NifTaggedEnum decoder ignores serde renames.
                        if v.is_array()
                            && let Some(enum_def) = enums.iter().find(|e| &e.name == elem_type && e.serde_tag.is_some())
                        {
                            let formatted = emit_tagged_enum_array(v, enum_def, enums);
                            if arg.optional {
                                parts.push(format!("{}: {formatted}", arg.name));
                            } else {
                                parts.push(formatted);
                            }
                            continue;
                        }
                        // When element_type is set to a simple type (e.g. Vec<String>).
                        // The NIF accepts an Elixir list directly — emit one.
                        if v.is_array() {
                            let formatted = json_to_elixir(v);
                            if arg.optional {
                                parts.push(format!("{}: {formatted}", arg.name));
                            } else {
                                parts.push(formatted);
                            }
                            continue;
                        }
                    }
                    // When there's no options_type+options_via, the Elixir NIF expects a JSON
                    // string (Option<String> decoded by serde_json) rather than an Elixir map.
                    // Serialize the JSON value to a string literal here.
                    // Emit as positional or keyword based on trailing optional arg count.
                    // If 2+ trailing optional args exist, use keyword form to avoid mixing
                    // positional args after keyword args. Otherwise, stay positional for
                    // compatibility with positional-default style facades.
                    if !v.is_null() {
                        let json_str = serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string());
                        let escaped = escape_elixir(&json_str);
                        let formatted = format!("\"{escaped}\"");
                        if use_keyword_form_for_optional_args && arg.optional {
                            parts.push(format!("{}: {formatted}", arg.name));
                        } else {
                            parts.push(formatted);
                        }
                        continue;
                    }
                }
                // Optional args use keyword-opts form: `name: value`.
                let elixir_val = json_to_elixir(v);
                if arg.optional {
                    parts.push(format!("{}: {elixir_val}", arg.name));
                } else {
                    parts.push(elixir_val);
                }
            }
        }
    }

    // Elixir requires all positional args before keyword args.
    // Separate positional and keyword args, preserving order within each group.
    // With the keyword-opts threshold applied above (use_keyword_form_for_optional_args),
    // we should never encounter a positional arg after a keyword arg.
    let mut positional_args = Vec::new();
    let mut keyword_args = Vec::new();

    for part in parts {
        let is_keyword = part.contains(": ") && !part.starts_with('"');
        if is_keyword {
            keyword_args.push(part);
        } else {
            positional_args.push(part);
        }
    }

    let mut final_args = positional_args;
    final_args.extend(keyword_args);

    (setup_lines, final_args.join(", "))
}

/// Returns true if the field expression is a numeric/integer expression
/// (e.g., a `length(...)` call) rather than a string.
fn is_numeric_expr(field_expr: &str) -> bool {
    field_expr.starts_with("length(")
}

#[allow(clippy::too_many_arguments)]
fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    module_path: &str,
    fields_enum: &std::collections::HashSet<String>,
    per_call_enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    is_streaming: bool,
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
            "chunks_have_heading_context" => {
                let pred = format!(
                    "Enum.all?({result_var}.chunks || [], fn c -> c.metadata != nil and c.metadata.heading_context != nil end)"
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
            "first_chunk_starts_with_heading" => {
                let expr = format!(
                    "case List.first({result_var}.chunks || []) do
        c when is_map(c) -> String.trim_leading(c.content || \"\") |> String.starts_with?(\"#\")
        _ -> false
      end"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "      assert ({expr})");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "      refute ({expr})");
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

    // Streaming virtual fields: intercept before is_valid_for_result so they are
    // never skipped.  These fields resolve against the `chunks` collected-list variable.
    if is_streaming {
        if let Some(f) = &assertion.field {
            if !f.is_empty() && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) {
                if let Some(expr) =
                    crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor(f, "elixir", result_var)
                {
                    match assertion.assertion_type.as_str() {
                        "count_min" => {
                            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                                let _ = writeln!(out, "      assert length({expr}) >= {n}");
                            }
                        }
                        "count_equals" => {
                            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                                let _ = writeln!(out, "      assert length({expr}) == {n}");
                            }
                        }
                        "equals" => {
                            if let Some(serde_json::Value::String(s)) = &assertion.value {
                                let escaped = escape_elixir(s);
                                let _ = writeln!(out, "      assert {expr} == \"{escaped}\"");
                            } else if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                                let _ = writeln!(out, "      assert {expr} == {n}");
                            }
                        }
                        "not_empty" => {
                            let _ = writeln!(out, "      assert {expr} != []");
                        }
                        "is_empty" => {
                            let _ = writeln!(out, "      assert {expr} == []");
                        }
                        "is_true" => {
                            let _ = writeln!(out, "      assert {expr}");
                        }
                        "is_false" => {
                            let _ = writeln!(out, "      refute {expr}");
                        }
                        "greater_than" => {
                            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                                let _ = writeln!(out, "      assert {expr} > {n}");
                            }
                        }
                        "greater_than_or_equal" => {
                            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                                let _ = writeln!(out, "      assert {expr} >= {n}");
                            }
                        }
                        "contains" => {
                            if let Some(serde_json::Value::String(s)) = &assertion.value {
                                let escaped = escape_elixir(s);
                                let _ = writeln!(out, "      assert String.contains?({expr}, \"{escaped}\")");
                            }
                        }
                        _ => {
                            let _ = writeln!(
                                out,
                                "      # streaming field '{f}': assertion type '{}' not rendered",
                                assertion.assertion_type
                            );
                        }
                    }
                }
                return;
            }
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
    // Check if the field is exactly metadata.format (FormatMetadata struct; needs special display conversion)
    // Don't match on other fields like metadata.output_format (which is a plain string)
    let field_is_format_metadata = assertion
        .field
        .as_deref()
        .filter(|f| !f.is_empty())
        .is_some_and(|f| f == "metadata.format" || f.ends_with(".metadata.format"));
    let coerced_field_expr = if field_is_format_metadata {
        format!("alef_e2e_format_to_string({field_expr})")
    } else if field_is_enum {
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
                    // Binary uses byte_size, list uses length, string uses String.length
                    let _ = writeln!(
                        out,
                        "      assert (is_binary({field_expr}) && byte_size({field_expr}) >= {n}) || (is_list({field_expr}) && length({field_expr}) >= {n}) || (is_binary({field_expr}) == false && is_list({field_expr}) == false && String.length({field_expr}) >= {n})"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "      assert (is_binary({field_expr}) && byte_size({field_expr}) <= {n}) || (is_list({field_expr}) && length({field_expr}) <= {n}) || (is_binary({field_expr}) == false && is_list({field_expr}) == false && String.length({field_expr}) <= {n})"
                    );
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

/// Build an Elixir call expression for a `method_result` assertion on a sample_language result.
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
fn build_elixir_visitor(setup_lines: &mut Vec<String>, visitor_spec: &crate::e2e::fixture::VisitorSpec) -> String {
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
        CallbackAction::CustomTemplate { template, .. } => {
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
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
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

// ---------------------------------------------------------------------------
// Tagged-enum array emission for Rustler NifTaggedEnum
// ---------------------------------------------------------------------------

/// Apply a serde `rename_all` strategy to a PascalCase variant name to derive
/// the wire-format tag value used in fixture inputs.
fn apply_rename_all(name: &str, strategy: Option<&str>) -> String {
    use heck::{ToKebabCase, ToLowerCamelCase, ToShoutyKebabCase, ToShoutySnakeCase, ToUpperCamelCase};
    match strategy {
        Some("snake_case") | None => name.to_snake_case(),
        Some("camelCase") => name.to_lower_camel_case(),
        Some("PascalCase") => name.to_upper_camel_case(),
        Some("SCREAMING_SNAKE_CASE") | Some("UPPERCASE") => name.to_shouty_snake_case(),
        Some("kebab-case") => name.to_kebab_case(),
        Some("SCREAMING-KEBAB-CASE") => name.to_shouty_kebab_case(),
        Some("lowercase") => name.to_lowercase(),
        Some(_) => name.to_snake_case(),
    }
}

/// Match an input JSON value (string) against a unit-only enum and return the
/// corresponding Rustler atom literal (e.g. `:down`). Returns None if the enum
/// is not unit-only or the value does not match any variant.
fn match_unit_enum_atom(value: &serde_json::Value, enum_def: &crate::core::ir::EnumDef) -> Option<String> {
    let s = value.as_str()?;
    if enum_def.variants.iter().any(|v| !v.fields.is_empty()) {
        return None;
    }
    for variant in &enum_def.variants {
        let wire_tag = variant
            .serde_rename
            .clone()
            .unwrap_or_else(|| apply_rename_all(&variant.name, enum_def.serde_rename_all.as_deref()));
        if wire_tag == s {
            return Some(format!(":{}", variant.name.to_snake_case()));
        }
    }
    None
}

/// Emit an Elixir list literal of Rustler NifTaggedEnum tuples for an internally-tagged
/// enum array. Each element renders as `:variant_atom` (unit) or
/// `{:variant_atom, %{field_atom: value}}` (struct), with variant/field atoms derived
/// from the Rust names via snake_case (NifTaggedEnum ignores serde rename for atoms).
fn emit_tagged_enum_array(
    value: &serde_json::Value,
    enum_def: &crate::core::ir::EnumDef,
    all_enums: &[crate::core::ir::EnumDef],
) -> String {
    let arr = match value.as_array() {
        Some(a) => a,
        None => return json_to_elixir(value),
    };
    let tag_key = enum_def.serde_tag.as_deref().unwrap_or("type");
    let mut elements: Vec<String> = Vec::with_capacity(arr.len());
    for item in arr {
        let obj = match item.as_object() {
            Some(o) => o,
            None => {
                elements.push(json_to_elixir(item));
                continue;
            }
        };
        let tag_value = obj.get(tag_key).and_then(|v| v.as_str()).unwrap_or("");
        let matched = enum_def.variants.iter().find(|variant| {
            let wire_tag = variant
                .serde_rename
                .clone()
                .unwrap_or_else(|| apply_rename_all(&variant.name, enum_def.serde_rename_all.as_deref()));
            wire_tag == tag_value
        });
        let Some(variant) = matched else {
            elements.push(json_to_elixir(item));
            continue;
        };
        let variant_atom = format!(":{}", variant.name.to_snake_case());
        if variant.fields.is_empty() {
            elements.push(variant_atom);
            continue;
        }
        let mut field_strs: Vec<String> = Vec::with_capacity(variant.fields.len());
        for field in &variant.fields {
            let wire_field = field.serde_rename.as_deref().unwrap_or(&field.name);
            let rust_field_atom = field.name.clone();
            let emitted_val = if let Some(field_val) = obj.get(wire_field) {
                // If the field's type is a Named reference to a unit-only enum, convert
                // the input string value to an atom via that enum's rename_all.
                if let crate::core::ir::TypeRef::Named(type_name) = &field.ty {
                    all_enums
                        .iter()
                        .find(|e| &e.name == type_name && e.serde_tag.is_none())
                        .and_then(|nested| match_unit_enum_atom(field_val, nested))
                        .unwrap_or_else(|| json_to_elixir(field_val))
                } else {
                    json_to_elixir(field_val)
                }
            } else if field.optional {
                // Optional fields missing from the JSON should use `nil` as default
                "nil".to_string()
            } else {
                // Non-optional fields missing from the JSON should not be included
                // (could indicate an error in the fixture, but we skip for safety)
                continue;
            };
            field_strs.push(format!("{rust_field_atom}: {emitted_val}"));
        }
        let map_body = field_strs.join(", ");
        elements.push(format!("{{{variant_atom}, %{{{map_body}}}}}"));
    }
    format!("[{}]", elements.join(", "))
}

/// Emit an Elixir test backend stub module for a trait bridge.
///
/// Generates a `defmodule TestStub{PascalId}` that implements the trait's required
/// methods using language-appropriate default return values. The stub is registered
/// via the trait bridge's `register_fn`.
/// Emit the Elixir GenServer stub that implements a trait bridge for testing.
///
/// `nif_module` is the Elixir module that exposes `complete_trait_call/2` and
/// `fail_trait_call/2` NIFs (e.g. `"MyApp.Native"` for a crate named `my_app`).
/// Pass an empty string to use the conventional `Native` fallback.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    nif_module: &str,
) -> super::TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use heck::ToUpperCamelCase;
    use std::fmt::Write as _;

    let pascal_id = fixture.id.to_upper_camel_case();
    let module_name = format!("TestStub{pascal_id}");

    // Resolve the NIF module that exposes complete_trait_call/2.
    // Falls back to "Native" when no explicit module is provided, which is
    // correct for standalone e2e fixtures not tied to a specific crate namespace.
    let effective_nif_module = if nif_module.is_empty() { "Native" } else { nif_module };

    // Derive the plugin name from the first argument's input field structure.
    // For "register_document_extractor_trait_bridge" with input { extractor: { name: "test-extractor" } },
    // we need to extract input.extractor.name.
    // Pattern: fixture.input has a single key (the argument name), which is an object containing "name".
    let plugin_name = fixture
        .input
        .as_object()
        .and_then(|obj| obj.values().next()) // Get the first value (should be the argument object)
        .and_then(|arg_obj| arg_obj.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or(&fixture.id)
        .to_string();

    let defaults = language_defaults("elixir");

    // Use a fully-qualified E2e.TestStubs namespace so the module name is unique
    // and well-scoped. Guard the definition with Code.ensure_loaded?/1 so that
    // re-running the same compiled test file does not trigger a redefinition
    // warning (which becomes an error under --warnings-as-errors).
    let qualified_module = format!("E2e.TestStubs.{module_name}");
    let genserver_module = format!("{}GenServer", qualified_module);

    // Emit module-level definitions (no leading spaces).
    let mut module_defs = String::new();
    let _ = writeln!(module_defs, "unless Code.ensure_loaded?({qualified_module}) do");
    let _ = writeln!(module_defs, "defmodule {qualified_module} do");

    // If there is a Plugin super-trait, emit `name/0`.
    if trait_bridge.super_trait.is_some() {
        let _ = writeln!(module_defs, "  def name, do: \"{plugin_name}\"");
        // initialize/0 has a Rust default impl but Rustler calls it unconditionally on
        // every registered plugin object — the Elixir stub must define it.
        let _ = writeln!(module_defs, "  def initialize, do: :ok");
    }

    // Emit required (non-default) methods.
    for method in methods {
        if method.has_default_impl {
            continue;
        }
        // Build parameter list: skip `self` receiver, emit param names.
        let params: Vec<&str> = method.params.iter().map(|p| p.name.as_str()).collect();
        let params_str = params.join(", ");

        // Special case: EmbeddingBackend.dimensions must return > 0, not 0.
        let default_val = if method.name == "dimensions" && trait_bridge.trait_name == "EmbeddingBackend" {
            "1".to_string()
        } else {
            defaults.emit_default(&method.return_type)
        };

        // Elixir NIFs that may error wrap the result in `{:ok, value}`.
        let return_expr = if method.error_type.is_some() {
            format!("{{:ok, {default_val}}}")
        } else {
            default_val
        };

        if params_str.is_empty() {
            let _ = writeln!(module_defs, "  def {}, do: {return_expr}", method.name);
        } else {
            let _ = writeln!(module_defs, "  def {}({params_str}), do: {return_expr}", method.name);
        }
    }

    let _ = writeln!(module_defs, "end");
    let _ = writeln!(module_defs, "end");

    // Emit the GenServer wrapper that Rustler NIFs can call via PID message passing.
    // Messages arrive as {:trait_call, method_atom, args_json_string, reply_id}.
    // The GenServer calls the stub module method, serializes the result to JSON, and
    // passes it back to the NIF's complete_trait_call/2 which unblocks the waiting Rust thread.
    let _ = writeln!(module_defs, "unless Code.ensure_loaded?({genserver_module}) do");
    let _ = writeln!(module_defs, "defmodule {genserver_module} do");
    let _ = writeln!(module_defs, "  use GenServer");
    let _ = writeln!(module_defs);
    let _ = writeln!(module_defs, "  def start_link(_opts) do");
    let _ = writeln!(module_defs, "    GenServer.start_link(__MODULE__, nil)");
    let _ = writeln!(module_defs, "  end");
    let _ = writeln!(module_defs);
    let _ = writeln!(module_defs, "  @impl true");
    let _ = writeln!(module_defs, "  def init(_), do: {{:ok, nil}}");
    let _ = writeln!(module_defs);
    let _ = writeln!(module_defs, "  @impl true");
    let _ = writeln!(
        module_defs,
        "  def handle_info({{:trait_call, method_atom, args_json, reply_id}}, state) do"
    );
    let _ = writeln!(module_defs, "    args = Jason.decode!(args_json)");
    let _ = writeln!(module_defs, "    result = apply({qualified_module}, method_atom, args)");
    let _ = writeln!(module_defs, "    result_json = Jason.encode!(result)");
    let _ = writeln!(
        module_defs,
        "    {effective_nif_module}.complete_trait_call(reply_id, result_json)"
    );
    let _ = writeln!(module_defs, "    {{:noreply, state}}");
    let _ = writeln!(module_defs, "  end");
    let _ = writeln!(module_defs, "end");
    let _ = writeln!(module_defs, "end");

    // Emit the test-function-level code: start the GenServer and capture its PID.
    // This will be indented when rendered inside the test function.
    let pid_var = format!("{}_pid", pascal_id.to_lowercase());
    let mut test_setup = String::new();
    let _ = writeln!(test_setup, "{{:ok, {pid_var}}} = {genserver_module}.start_link(nil)");

    // Combine both parts with a separator so we can split them during rendering.
    // Use `\n__TRAIT_BRIDGE_MODULE_DEFS_END__\n` as a marker.
    let mut combined_setup = module_defs;
    combined_setup.push_str("\n__TRAIT_BRIDGE_MODULE_DEFS_END__\n");
    combined_setup.push_str(&test_setup);

    super::TestBackendEmission {
        setup_block: combined_setup,
        arg_expr: pid_var,
        type_imports: Vec::new(),
    }
}

/// Extract the backend name from fixture input for register_fn traits.
///
/// Looks for a "name" field at the root or nested one level deep,
/// then falls back to the first string value encountered, then to the fallback.
fn extract_backend_name_from_input(input: &serde_json::Value, fallback: &str) -> String {
    if let Some(obj) = input.as_object() {
        if let Some(s) = obj.get("name").and_then(|v| v.as_str()) {
            return s.to_string();
        }
        for v in obj.values() {
            if let Some(inner) = v.as_object() {
                if let Some(s) = inner.get("name").and_then(|v| v.as_str()) {
                    return s.to_string();
                }
            }
        }
        for v in obj.values() {
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
        }
    }
    fallback.to_string()
}

#[cfg(test)]
mod test_backend_tests {
    use super::emit_test_backend;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, PrimitiveType, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn make_trait_bridge(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
            ..Default::default()
        }
    }

    fn make_method(name: &str, required: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: !required,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    fn make_fixture(id: &str) -> Fixture {
        Fixture {
            id: id.to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            assertions: vec![],
            visitor: None,
            args: vec![],
        }
    }

    /// Verify that no sample_core-domain names leak into the generated output when
    /// the trait bridge is configured for a synthetic `TestTrait` in `testlib`.
    #[test]
    fn elixir_stub_contains_no_sample_crate_domain_names() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");

        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            !output.contains("SampleCrate"),
            "must not contain literal 'SampleCrate', got:\n{output}"
        );
        assert!(
            !output.contains("sample_crate::"),
            "must not contain 'sample_crate::', got:\n{output}"
        );
        assert!(
            !output.contains("SampleCrateBridge"),
            "must not contain 'SampleCrateBridge', got:\n{output}"
        );
        assert!(
            output.contains("TestStubMyTestFixture"),
            "module name must be derived from fixture id, got:\n{output}"
        );
        assert!(
            output.contains("def process"),
            "required method 'process' must be emitted, got:\n{output}"
        );
    }

    /// Verify that the defmodule is guarded with `Code.ensure_loaded?` to prevent
    /// redefinition warnings when the same compiled test file is loaded multiple times.
    #[test]
    fn elixir_stub_defmodule_guarded_against_redefinition() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");
        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            output.contains("unless Code.ensure_loaded?"),
            "defmodule must be guarded with `unless Code.ensure_loaded?` to prevent redefine warnings, got:\n{output}"
        );
        // The module atom in the `unless` guard must match the arg_expr.
        assert!(
            emission.setup_block.contains(&emission.arg_expr),
            "setup_block must reference the same module atom as arg_expr, got:\narg_expr={}\nsetup_block={}",
            emission.arg_expr,
            emission.setup_block
        );
    }

    /// Verify that `fixture.input.<arg>.name` is used as the plugin name when present.
    /// Fixture structure: { "backend": { "name": "my-backend-name" } }
    #[test]
    fn elixir_stub_uses_fixture_input_name_for_plugin_name() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process", true);
        let methods = [&required_method];
        let mut fixture = make_fixture("my_fixture_id");
        fixture.input = serde_json::json!({ "backend": { "name": "my-backend-name" } });

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");
        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            output.contains("\"my-backend-name\""),
            "plugin name must come from fixture.input.<arg>.name, got:\n{output}"
        );
    }

    /// Verify that the module is emitted under the E2e.TestStubs namespace so it is
    /// well-scoped and does not pollute the top-level Elixir module namespace.
    #[test]
    fn elixir_stub_uses_scoped_namespace() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");

        assert!(
            emission.setup_block.contains("E2e.TestStubs."),
            "setup_block must reference E2e.TestStubs namespace, got:\n{}",
            emission.setup_block
        );
    }

    /// Verify that a GenServer is emitted to wrap the stub module so Rustler NIFs
    /// can call trait methods via PID message passing.
    #[test]
    fn elixir_stub_emits_genserver_wrapper() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");

        assert!(
            emission.setup_block.contains("defmodule") && emission.setup_block.contains("GenServer"),
            "setup_block must define a GenServer module, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("handle_info"),
            "GenServer must implement handle_info for trait_call messages, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("complete_trait_call"),
            "GenServer must reply via the NIF complete_trait_call/2, got:\n{}",
            emission.setup_block
        );
    }

    /// Verify that arg_expr is a PID variable, not a module name.
    /// This allows Rustler NIFs to receive the PID and send messages to it.
    #[test]
    fn elixir_stub_arg_expr_is_pid_variable() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");

        // arg_expr should be a lowercase variable name like "my_test_fixture_pid", not a module atom
        assert!(
            !emission.arg_expr.contains("."),
            "arg_expr must be a PID variable (not a module atom), got:\n{}",
            emission.arg_expr
        );
        assert!(
            emission.arg_expr.ends_with("_pid"),
            "arg_expr must end with _pid to indicate it is a process identifier, got:\n{}",
            emission.arg_expr
        );
        assert!(
            emission
                .setup_block
                .contains(&format!("{{:ok, {}}}", emission.arg_expr)),
            "setup_block must start GenServer and assign its PID to the arg_expr variable, got:\n{}",
            emission.setup_block
        );
    }
}
