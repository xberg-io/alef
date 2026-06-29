use crate::core::template_versions as tv;
use crate::e2e::config::E2eConfig;
use std::fmt::Write as FmtWrite;

// render_app_harness has been moved to a consumer extension
// (the e2e extension). Do not add it back here.

/// Emit an Elixir snippet that sets every `[e2e.env]` entry into the environment
/// using `System.get_env` to check first so a parent runner can override at spawn time.
/// Returns empty when no env vars are configured.
fn render_env_setup_block(e2e_config: &E2eConfig) -> String {
    if e2e_config.env.is_empty() {
        return String::new();
    }
    let mut keys: Vec<&String> = e2e_config.env.keys().collect();
    keys.sort();
    let mut out = String::new();
    for k in keys {
        let v = &e2e_config.env[k];
        out.push_str(&format!(
            "unless System.get_env(\"{}\") do\n  System.put_env(\"{}\", \"{}\")\nend\n",
            k, k, v
        ));
    }
    out.push('\n');
    out
}

/// Resolve the Elixir NIF module that exposes `set_env/2`.
///
/// Mirrors the module-path resolution used elsewhere in the Elixir e2e codegen
/// (see `test_case.rs`): a value containing `.` or starting uppercase is used
/// verbatim; otherwise it is PascalCased. The NIF lives under `<Module>.Native`.
fn elixir_nif_module(e2e_config: &E2eConfig) -> String {
    let raw_module = &e2e_config.call.module;
    let module_path = if raw_module.contains('.') || raw_module.chars().next().is_some_and(|c| c.is_uppercase()) {
        raw_module.clone()
    } else {
        super::values::elixir_module_name(raw_module)
    };
    format!("{module_path}.Native")
}

/// Emit the native env-propagation block and the test-documents chdir.
///
/// Elixir's `System.put_env` (Erlang `os:putenv`) does not reach the native C
/// runtime that an FFI library reads via `getenv`, so each configured `[e2e.env]`
/// entry is also pushed through the binding's `set_env/2` NIF (libc setenv),
/// wrapped in `try/rescue` so suites whose binding lacks the NIF still load.
///
/// The chdir runs from `test_documents_dir` so relative file URIs resolve,
/// mirroring the Python/Go suites. Returns an empty string when no `[e2e.env]`
/// entries are configured — the native propagation and the chdir are only needed
/// by suites that opt into env-driven setup (and the empty case keeps the
/// generated `test_helper.exs` free of any `System.get_env` references).
fn render_native_env_and_chdir(e2e_config: &E2eConfig) -> String {
    if e2e_config.env.is_empty() {
        return String::new();
    }

    let mut out = String::new();

    let nif_module = elixir_nif_module(e2e_config);
    out.push_str("# Erlang os:putenv does not propagate to the native C runtime that an FFI\n");
    out.push_str("# library reads via getenv. Push each value through the binding's set_env NIF\n");
    out.push_str("# (libc setenv) so the native side observes the same environment.\n");
    let mut keys: Vec<&String> = e2e_config.env.keys().collect();
    keys.sort();
    for k in keys {
        let v = &e2e_config.env[k];
        out.push_str("try do\n");
        let _ = writeln!(out, "  {nif_module}.set_env(\"{k}\", \"{v}\")");
        out.push_str("rescue\n");
        out.push_str("  _ -> :ok\n");
        out.push_str("end\n");
    }
    out.push('\n');

    let rel = e2e_config.test_documents_relative_from(1);
    out.push_str("# Run from the test-documents dir so relative file URIs (e.g. \"text/report.txt\")\n");
    out.push_str("# resolve, mirroring the other language suites which chdir before running.\n");
    out.push_str("test_documents_dir =\n");
    let _ = writeln!(
        out,
        "  System.get_env(\"ALEF_TEST_DOCUMENTS_DIR\") || Path.expand(\"{rel}\", __DIR__)"
    );
    out.push('\n');
    out.push_str("if File.dir?(test_documents_dir), do: File.cd!(test_documents_dir)\n\n");

    out
}

/// Render the non-server-pattern `test/test_helper.exs`.
///
/// Covers the mock-server case (`has_http_tests` = true) and the NIF-only case.
/// The server-pattern (`uses_harness`) branch is owned by a consumer extension
/// via `Extension::emit_e2e`.
pub(super) fn render_test_helper(has_http_tests: bool, e2e_config: &E2eConfig) -> String {
    // Environment variables (including E2E_ALLOW_PRIVATE_NETWORK) must be set BEFORE
    // the Rustler NIF loads (at first module init). render_env_setup_block emits all [e2e.env]
    // vars with System.get_env guards, ensuring they're set early.
    let env_setup = render_env_setup_block(e2e_config);
    let native_and_chdir = render_native_env_and_chdir(e2e_config);

    if has_http_tests {
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
        format!("{}{}{}{}", env_setup, native_and_chdir, finch_setup, mock_server)
    } else {
        format!("{}{}ExUnit.start()\n", env_setup, native_and_chdir)
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
