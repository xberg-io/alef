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

    /// The `has_http_tests=true` (mock-server) path must start a named Finch pool
    /// before `ExUnit.start()` so that `Req` calls with `finch: AlefE2EFinch` work.
    ///
    /// Note: the server-pattern (`uses_harness`) test_helper is emitted by a
    /// consumer extension via `Extension::emit_e2e`, not here.
    #[test]
    fn test_helper_http_path_includes_named_finch_supervisor() {
        let config = make_e2e_config();
        let output = render_test_helper(true, &config);
        assert!(
            output.contains("Finch.start_link(name: AlefE2EFinch, pools: %{:default => [protocols: [:http1]]})"),
            "has_http_tests path must start named Finch pool, got:\n{output}"
        );
        assert!(
            output.contains("ExUnit.start()"),
            "has_http_tests path must call ExUnit.start(), got:\n{output}"
        );
    }

    /// Env vars from [e2e.env] must appear before ExUnit.start() to ensure
    /// the binding's first call sees the configured environment.
    #[test]
    fn test_helper_emits_env_setup_before_exunit() {
        let mut config = make_e2e_config();
        config
            .env
            .insert("ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());
        config.env.insert("DEBUG_MODE".to_string(), "false".to_string());

        let output = render_test_helper(false, &config);

        // Must contain System.put_env calls for both keys
        assert!(
            output.contains("System.get_env(\"ALLOW_PRIVATE_NETWORK\")"),
            "must check ALLOW_PRIVATE_NETWORK, got:\n{output}"
        );
        assert!(
            output.contains("System.put_env(\"ALLOW_PRIVATE_NETWORK\", \"true\")"),
            "must set ALLOW_PRIVATE_NETWORK, got:\n{output}"
        );
        assert!(
            output.contains("System.get_env(\"DEBUG_MODE\")"),
            "must check DEBUG_MODE, got:\n{output}"
        );
        assert!(
            output.contains("System.put_env(\"DEBUG_MODE\", \"false\")"),
            "must set DEBUG_MODE, got:\n{output}"
        );

        // Keys must appear in alphabetical order
        let allow_pos = output
            .find("ALLOW_PRIVATE_NETWORK")
            .expect("ALLOW_PRIVATE_NETWORK not found");
        let debug_pos = output.find("DEBUG_MODE").expect("DEBUG_MODE not found");
        assert!(
            allow_pos < debug_pos,
            "env vars must be sorted alphabetically, got:\n{output}"
        );

        // Env setup must come before ExUnit.start()
        let env_setup_end = output.rfind("System.put_env").expect("no System.put_env found");
        let exunit_start = output.find("ExUnit.start()").expect("ExUnit.start() not found");
        assert!(
            env_setup_end < exunit_start,
            "env setup must come before ExUnit.start(), got:\n{output}"
        );
    }

    /// When [e2e.env] is empty, no env setup block should be emitted.
    #[test]
    fn test_helper_empty_env_produces_no_setup_block() {
        let config = make_e2e_config();
        // config.env is empty by default

        let output = render_test_helper(false, &config);

        // Should not contain System.put_env or System.get_env
        assert!(
            !output.contains("System.put_env"),
            "empty env must not emit System.put_env, got:\n{output}"
        );
        assert!(
            !output.contains("System.get_env"),
            "empty env must not emit System.get_env, got:\n{output}"
        );
        // But should still contain ExUnit.start()
        assert!(
            output.contains("ExUnit.start()"),
            "must still start ExUnit, got:\n{output}"
        );
    }
}
