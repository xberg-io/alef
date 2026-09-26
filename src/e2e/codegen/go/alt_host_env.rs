//! Split out of `go.rs` (already at the file-modularization cap; see CLAUDE.md's
//! `file-modularization` rule) so this new behavior does not push it further over.

use std::fmt::Write as FmtWrite;

/// Emit the `main_test.go` lines that bake a configured `[crates.e2e] alt_host` into the
/// standalone mock-server spawn's env. `LookupEnv`-guarded so an already-exported override
/// (e.g. set by a parent `alef test-apps run`) wins over the configured default. ~keep
pub(super) fn render_alt_host_env_lines(out: &mut String, alt_host: &str) {
    let _ = writeln!(out, "\tif _, ok := os.LookupEnv(\"ALEF_MOCK_ALT_HOST\"); !ok {{");
    let _ = writeln!(out, "\t\tcmdEnv = append(cmdEnv, \"ALEF_MOCK_ALT_HOST={alt_host}\")");
    let _ = writeln!(out, "\t}}");
}

#[cfg(test)]
mod tests {
    use super::render_alt_host_env_lines;

    /// A distinctive alt_host value (not the crate-default `"localhost"`) proves the emitted
    /// lines carry the configured value through rather than hard-coding it.
    #[test]
    fn bakes_the_configured_alt_host_lookup_env_guarded() {
        let mut out = String::new();
        render_alt_host_env_lines(&mut out, "alt-host-from-config.test");
        assert!(
            out.contains("cmdEnv = append(cmdEnv, \"ALEF_MOCK_ALT_HOST=alt-host-from-config.test\")"),
            "got:\n{out}"
        );
        assert!(
            out.contains("os.LookupEnv(\"ALEF_MOCK_ALT_HOST\")"),
            "must guard on LookupEnv so an already-exported override wins, got:\n{out}"
        );
    }
}
