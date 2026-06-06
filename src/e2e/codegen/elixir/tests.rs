mod mix_exs_tests {
    use super::super::project::render_mix_exs;
    use crate::e2e::config::DependencyMode;

    /// When a project has mock-server fixtures (e.g. via `mock_response` or
    /// `client_factory`) but no raw HTTP fixtures and no server-pattern
    /// harness, render_test_helper still emits `Finch.start_link(...)`. The
    /// corresponding mix.exs MUST declare the Finch + Req + Jason deps,
    /// otherwise `mix test` fails with
    /// `(UndefinedFunctionError) function Finch.start_link/1 is undefined`.
    #[test]
    fn mix_exs_includes_finch_when_mock_server_tests_are_present() {
        let output = render_mix_exs(
            "sample_service",
            "1.4.0-rc.55",
            DependencyMode::Registry,
            false, // has_http_tests
            true,  // has_mock_server_tests
            true,  // has_nif_tests
            false, // uses_harness
        );
        assert!(
            output.contains(":finch,"),
            "mock-server-only project must declare :finch dep, got:\n{output}"
        );
        assert!(
            output.contains(":req,"),
            "mock-server-only project must declare :req dep, got:\n{output}"
        );
        assert!(
            output.contains(":jason,"),
            "mock-server-only project must declare :jason dep, got:\n{output}"
        );
    }

    /// Pure-NIF project (no HTTP, no mock-server, no harness) must NOT pull
    /// in Finch/Req/Jason - keeps the dep graph minimal for offline tests.
    #[test]
    fn mix_exs_omits_finch_when_no_http_or_mock_server_tests() {
        let output = render_mix_exs(
            "sample_service",
            "../../packages/elixir",
            DependencyMode::Local,
            false, // has_http_tests
            false, // has_mock_server_tests
            true,  // has_nif_tests
            false, // uses_harness
        );
        assert!(
            !output.contains(":finch,"),
            "pure-NIF project must not declare :finch dep, got:\n{output}"
        );
    }
}

mod test_helper_tests {
    use super::super::project::render_test_helper;
    use crate::e2e::config::E2eConfig;

    fn make_e2e_config() -> E2eConfig {
        E2eConfig::default()
    }

    /// The `uses_harness=true` path (server-pattern) must start a named Finch pool
    /// before `ExUnit.start()` so that `Req` calls with `finch: AlefE2EFinch` work.
    #[test]
    fn test_helper_harness_path_includes_named_finch_supervisor() {
        let config = make_e2e_config();
        let output = render_test_helper(false, true, &config);
        assert!(
            output.contains("Finch.start_link(name: AlefE2EFinch)"),
            "uses_harness path must start named Finch pool, got:\n{output}"
        );
        assert!(
            output.contains("ExUnit.start()"),
            "uses_harness path must call ExUnit.start(), got:\n{output}"
        );
    }

    /// The `has_http_tests=true` (mock-server) path must also start a named Finch pool
    /// so that `Req` calls with `finch: AlefE2EFinch` work.
    #[test]
    fn test_helper_http_path_includes_named_finch_supervisor() {
        let config = make_e2e_config();
        let output = render_test_helper(true, false, &config);
        assert!(
            output.contains("Finch.start_link(name: AlefE2EFinch)"),
            "has_http_tests path must start named Finch pool, got:\n{output}"
        );
        assert!(
            output.contains("ExUnit.start()"),
            "has_http_tests path must call ExUnit.start(), got:\n{output}"
        );
    }
}
