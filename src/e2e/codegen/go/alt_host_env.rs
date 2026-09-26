//! Split out of `go.rs` (already at the file-modularization cap; see CLAUDE.md's
//! `file-modularization` rule) so this behavior does not push it further over.

/// Emit the `main_test.go` lines that assemble the standalone mock-server spawn's `cmdEnv`:
/// the configured `[crates.e2e] alt_host` override, followed by one `Getenv` forwarding block
/// per configured `env` key. Both are rendered through a single Jinja template
/// (`go/mock_server_spawn_env.go.jinja`) rather than two separate call sites — converting only
/// one of this block's two interpolation sites to a template while leaving the other as
/// `writeln!` would be worse than the status quo (see the `jinja-templates` rule and issue
/// #445), so they are threaded through together as one unit. ~keep
///
/// The `alt_host` block is `LookupEnv`-guarded so an already-exported override (e.g. set by a
/// parent `alef test-apps run`) wins over the configured default. The `env` loop re-reads each
/// configured key via `os.Getenv` rather than trusting the earlier `os.Setenv` in `TestMain`:
/// `os.Setenv` only affects Go's own runtime env, not libc's, and the mock-server (a C FFI
/// process) reads the libc environment directly. `env` is a `BTreeMap`, so `env.keys()` already
/// iterates in key order -- no separate sort needed to keep generation reproducible. ~keep
pub(super) fn render_mock_server_spawn_env(
    out: &mut String,
    alt_host: &str,
    env: &std::collections::BTreeMap<String, String>,
) {
    let env_keys: Vec<&str> = env.keys().map(String::as_str).collect();
    out.push_str(&crate::e2e::template_env::render(
        "go/mock_server_spawn_env.go.jinja",
        minijinja::context! { alt_host => alt_host, env_keys => env_keys },
    ));
}

#[cfg(test)]
mod tests {
    use super::render_mock_server_spawn_env;

    /// A distinctive alt_host value (not the crate-default `"localhost"`) proves the emitted
    /// lines carry the configured value through rather than hard-coding it.
    #[test]
    fn bakes_the_configured_alt_host_lookup_env_guarded() {
        let mut out = String::new();
        render_mock_server_spawn_env(&mut out, "alt-host-from-config.test", &Default::default());
        assert!(
            out.contains("cmdEnv = append(cmdEnv, \"ALEF_MOCK_ALT_HOST=alt-host-from-config.test\")"),
            "got:\n{out}"
        );
        assert!(
            out.contains("os.LookupEnv(\"ALEF_MOCK_ALT_HOST\")"),
            "must guard on LookupEnv so an already-exported override wins, got:\n{out}"
        );
    }

    /// Configured `env` keys must be forwarded via a `Getenv`-guarded append, in sorted order
    /// (the caller passes a `BTreeMap`), after the alt_host block.
    #[test]
    fn forwards_env_keys_after_the_alt_host_block_in_sorted_order() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("ZEBRA".to_string(), "z".to_string());
        env.insert("ALPHA".to_string(), "a".to_string());
        let mut out = String::new();
        render_mock_server_spawn_env(&mut out, "localhost", &env);

        let alt_host_idx = out.find("ALEF_MOCK_ALT_HOST").expect("must emit alt_host block");
        let alpha_idx = out
            .find("cmdEnv = append(cmdEnv, \"ALPHA=\"+v)")
            .expect("must forward ALPHA");
        let zebra_idx = out
            .find("cmdEnv = append(cmdEnv, \"ZEBRA=\"+v)")
            .expect("must forward ZEBRA");
        assert!(
            alt_host_idx < alpha_idx && alpha_idx < zebra_idx,
            "expected alt_host, then ALPHA, then ZEBRA in order; got:\n{out}"
        );
    }

    /// Empty `env` must not leave a dangling blank line or stray loop artifact after the
    /// alt_host block.
    #[test]
    fn empty_env_emits_only_the_alt_host_block() {
        let mut out = String::new();
        render_mock_server_spawn_env(&mut out, "localhost", &Default::default());
        assert_eq!(
            out,
            "\tif _, ok := os.LookupEnv(\"ALEF_MOCK_ALT_HOST\"); !ok {\n\t\tcmdEnv = append(cmdEnv, \"ALEF_MOCK_ALT_HOST=localhost\")\n\t}\n",
            "got:\n{out}"
        );
    }
}
