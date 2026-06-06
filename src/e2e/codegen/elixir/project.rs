use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::FixtureGroup;
use std::fmt::Write as FmtWrite;

use super::values::elixir_module_name;

pub(super) fn render_app_harness(
    e2e_config: &E2eConfig,
    groups: &[FixtureGroup],
    config: &ResolvedCrateConfig,
) -> String {
    // Collect all HTTP fixtures from all groups.
    let mut fixtures_map = serde_json::Map::new();

    for group in groups {
        for fixture in &group.fixtures {
            if fixture.http.is_none() {
                continue;
            }
            // Convert the fixture to JSON for the harness to load.
            // We only need the http field, handler, request, and expected_response.
            let http_data = &fixture.http.as_ref().unwrap();
            let fixture_json = serde_json::json!({
                "http": {
                    "handler": {
                        "route": &http_data.handler.route,
                        "method": &http_data.handler.method,
                        "body_schema": http_data.handler.body_schema.clone(),
                    },
                    "request": {
                        "path": &http_data.request.path,
                    },
                    "expected_response": {
                        "status_code": http_data.expected_response.status_code,
                        "body": &http_data.expected_response.body,
                        "headers": &http_data.expected_response.headers,
                    }
                }
            });
            fixtures_map.insert(fixture.id.clone(), fixture_json);
        }
    }

    let fixtures_json_str = serde_json::to_string(&fixtures_map).unwrap_or_default();
    // Escape backslashes and quotes for Elixir string literal
    let fixtures_json = fixtures_json_str.replace('\\', "\\\\").replace('"', "\\\"");
    let fixtures_json = format!("\"{}\"", fixtures_json);

    // Apply per-language harness overrides on top of the top-level harness config.
    let harness_override = e2e_config.harness.overrides.get("elixir");
    let imports_override = harness_override.and_then(|o| o.imports.as_ref());
    let imports: &[String] = imports_override.unwrap_or(&e2e_config.harness.imports);
    let app_class: Option<&str> = harness_override
        .and_then(|o| o.app_class.as_deref())
        .or(e2e_config.harness.app_class.as_deref());
    let register_route_method: Option<&str> = harness_override
        .and_then(|o| o.register_method.as_deref())
        .or(e2e_config.harness.register_method.as_deref());
    let body_schema_setter: Option<&str> = harness_override
        .and_then(|o| o.body_schema_setter.as_deref())
        .or(e2e_config.harness.body_schema_setter.as_deref());
    let run_method: Option<&str> = harness_override
        .and_then(|o| o.run_method.as_deref())
        .or(e2e_config.harness.run_method.as_deref());
    let host = &e2e_config.harness.host;
    let port = e2e_config.harness.port;

    let header = hash::header(CommentStyle::Hash);

    let binding_path = if e2e_config.dep_mode == crate::e2e::config::DependencyMode::Local {
        "../../packages/elixir"
    } else {
        "."
    };

    // Build module paths for RouteBuilder, Method, and App using the binding name from imports[0]
    let module_prefix = if !imports.is_empty() {
        format!("{}.", elixir_module_name(&imports[0]))
    } else {
        String::new()
    };
    let route_builder_class = format!("{}RouteBuilder", module_prefix);
    let method_enum_class = format!("{}Method", module_prefix);
    let server_config_class = format!("{}ServerConfig", module_prefix);
    let unqualified_app_class = app_class.unwrap_or("App");
    let app_class_name = format!("{}{}", module_prefix, unqualified_app_class);

    // Check if App.config is excluded from bindings. If excluded, the harness should not
    // call it and instead rely on default ServerConfig construction.
    let config_method_key = format!("{}.config", app_class_name);
    let skip_app_config = config.exclude.methods.iter().any(|m| m == &config_method_key);

    let ctx = minijinja::context! {
        header => header,
        app_class => app_class_name,
        route_builder_class => &route_builder_class,
        route_builder_schema_setter => body_schema_setter.unwrap_or("request_schema_json"),
        method_enum_class => &method_enum_class,
        register_route_method => register_route_method.unwrap_or("route"),
        run_method => run_method.unwrap_or("run"),
        server_config_class => &server_config_class,
        host => host,
        port => port,
        binding_path => binding_path,
        fixtures_json => fixtures_json,
        skip_app_config => skip_app_config,
    };

    crate::e2e::template_env::render("elixir/app_harness.exs.jinja", ctx)
}

pub(super) fn render_test_helper(has_http_tests: bool, uses_harness: bool, e2e_config: &E2eConfig) -> String {
    if uses_harness {
        // Server-pattern harness: spawn app_harness.exs subprocess
        let host = &e2e_config.harness.host;
        let port = e2e_config.harness.port;
        format!(
            r#"# Start a named Finch pool before ExUnit configured to use HTTP/1 only.
# Tests pass `finch: AlefE2EFinch` on every Req call; the pool's protocol
# selection (via `pools.default.protocols: [:http1]`) is the canonical place
# to pin the wire protocol since Req rejects per-call `:connect_options` when
# `:finch` is set.
{{:ok, _}} = Finch.start_link(name: AlefE2EFinch, pools: %{{:default => [protocols: [:http1]]}})

ExUnit.start()

# Spawn app_harness subprocess and set SUT_URL
# If SUT_URL is already set, a parent process started a shared harness.
# Use it as-is and do NOT spawn our own.

unless System.get_env("SUT_URL") do
  app_harness_bin = Path.expand("../app_harness.exs", __DIR__)
  project_root = Path.expand("..", __DIR__)

  # Build the list of ebin directories from _build/dev/lib so the harness can access compiled dependencies
  build_lib_dir = Path.join(project_root, "_build/dev/lib")
  lib_paths = if File.dir?(build_lib_dir) do
    File.ls!(build_lib_dir)
    |> Enum.map(&Path.join(build_lib_dir, &1))
    |> Enum.filter(&File.dir?/1)
    |> Enum.flat_map(fn lib_path ->
      ebin_path = Path.join(lib_path, "ebin")
      if File.dir?(ebin_path), do: ["-pa", ebin_path], else: []
    end)
  else
    []
  end

  # Use `elixir` to execute the harness script with proper code paths
  port = Port.open({{:spawn_executable, System.find_executable("elixir")}}, [
    :binary,
    {{:line, 65_536}},
    args: lib_paths ++ [app_harness_bin]
  ])

  url = "http://{host}:{port}"

  # Poll until the harness accepts TCP connections
  deadline = :erlang.monotonic_time(:millisecond) + 15_000
  ready = false

  {{ready, url}} =
    Enum.reduce_while(1..150, {{false, url}}, fn _, {{_, url_acc}} ->
      now = :erlang.monotonic_time(:millisecond)
      if now > deadline do
        {{:halt, {{false, url_acc}}}}
      else
        case :gen_tcp.connect(String.to_charlist("{host}"), {port}, [], 500) do
          {{:ok, socket}} ->
            :gen_tcp.close(socket)
            {{:halt, {{true, url_acc}}}}
          {{:error, _}} ->
            Process.sleep(100)
            {{:cont, {{false, url_acc}}}}
        end
      end
    end)

  unless ready do
    Port.close(port)
    raise "App harness did not become reachable on {host}:{port} within 15s"
  end

  System.put_env("SUT_URL", url)
end
"#
        )
    } else if has_http_tests {
        let finch_setup = r#"# Start a named Finch pool before ExUnit configured to use HTTP/1 only.
# Tests pass `finch: AlefE2EFinch` on every Req call; the pool's protocol
# selection (via `pools.default.protocols: [:http1]`) is the canonical place
# to pin the wire protocol since Req rejects per-call `:connect_options` when
# `:finch` is set.
{:ok, _} = Finch.start_link(name: AlefE2EFinch, pools: %{:default => [protocols: [:http1]]})

ExUnit.start()

"#;
        let mock_server =
            crate::e2e::template_env::render("elixir/test_helper_mock_server.exs.jinja", minijinja::context!());
        format!("{}{}", finch_setup, mock_server)
    } else {
        "ExUnit.start()\n".to_string()
    }
}

pub(super) fn render_mix_exs(
    pkg_name: &str,
    pkg_path: &str,
    dep_mode: crate::e2e::config::DependencyMode,
    has_http_tests: bool,
    has_mock_server_tests: bool,
    has_nif_tests: bool,
    uses_harness: bool,
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

    // Add Req + Jason + Finch for HTTP testing, mock-server testing, or
    // server-pattern harness. The test_helper.exs is emitted whenever any of
    // `has_http_tests`, `has_mock_server_tests`, or `uses_harness` is true
    // (see render_test_helper invocation) and always calls
    // Finch.start_link(name: AlefE2EFinch), so all three flags must trigger
    // dep inclusion. Without `has_mock_server_tests` here, projects that use
    // mock-server fixtures via `mock_response` / `client_factory` but no raw
    // http fixtures (and no harness) generate a test_helper.exs that calls
    // Finch without the corresponding mix dep, producing
    // `(UndefinedFunctionError) function Finch.start_link/1 is undefined`
    // at `mix test`.
    if has_http_tests || has_mock_server_tests || uses_harness {
        deps.push(format!("      {{:finch, \"{finch}\"}}", finch = tv::hex::FINCH));
        deps.push(format!("      {{:req, \"{req}\"}}", req = tv::hex::REQ));
        deps.push(format!("      {{:jason, \"{jason}\"}}", jason = tv::hex::JASON));
    }

    let _ = writeln!(out, "{}", deps.join(",\n"));
    let _ = writeln!(out, "    ]");
    let _ = writeln!(out, "  end");
    let _ = writeln!(out, "end");
    out
}
