//! Rendering for `.cargo/config.toml` so the Rust e2e suite injects every
//! `[e2e.env]` entry into every `cargo test`-spawned process before any
//! binding's engine is constructed.
//!
//! Cargo's `[env]` table applies to all child processes spawned by cargo
//! (including each integration-test binary). With `force = false` (the
//! default), pre-existing environment values from the parent shell are
//! preserved — matching the `setdefault` semantics required by the
//! `[e2e.env]` contract.

use crate::core::hash::{self, CommentStyle};
use std::collections::BTreeMap;
use std::fmt::Write as FmtWrite;

/// Render `.cargo/config.toml` with an `[env]` block populated from
/// `[e2e.env]`. Returns `None` when the env map is empty so the emitter can
/// skip writing the file entirely.
///
/// Each entry uses `{ value = "...", force = false }` so values from the parent shell win
/// (setdefault semantics). `env` is a `BTreeMap`, so iteration is already in key order for
/// deterministic output. ~keep
pub fn render_cargo_config(env: &BTreeMap<String, String>) -> Option<String> {
    if env.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out);
    let _ = writeln!(out, "# Suite-level environment defaults from [e2e.env].");
    let _ = writeln!(
        out,
        "# `force = false` preserves any value already set by the parent shell,"
    );
    let _ = writeln!(
        out,
        "# matching the setdefault semantics required by the [e2e.env] contract."
    );
    let _ = writeln!(out, "[env]");
    for (key, value) in env {
        // Escape backslashes and double quotes for TOML basic strings.
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        let _ = writeln!(out, "{key} = {{ value = \"{escaped}\", force = false }}");
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_cargo_config_returns_none_when_env_empty() {
        let env = BTreeMap::new();
        assert!(render_cargo_config(&env).is_none());
    }

    #[test]
    fn render_cargo_config_emits_env_table_with_sorted_keys() {
        let mut env = BTreeMap::new();
        env.insert("E2E_ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());
        env.insert("ALEF_FOO".to_string(), "bar".to_string());
        let out = render_cargo_config(&env).expect("non-empty env yields config");
        assert!(out.contains("[env]"), "got: {out}");
        assert!(
            out.contains("ALEF_FOO = { value = \"bar\", force = false }"),
            "got: {out}"
        );
        assert!(
            out.contains("E2E_ALLOW_PRIVATE_NETWORK = { value = \"true\", force = false }"),
            "got: {out}"
        );
        let alef_pos = out.find("ALEF_FOO").unwrap();
        let e2e_pos = out.find("E2E_ALLOW_PRIVATE_NETWORK").unwrap();
        assert!(alef_pos < e2e_pos, "keys must be sorted alphabetically; got: {out}");
    }

    #[test]
    fn render_cargo_config_escapes_quotes_and_backslashes() {
        let mut env = BTreeMap::new();
        env.insert("HAS_QUOTES".to_string(), "a\"b\\c".to_string());
        let out = render_cargo_config(&env).expect("non-empty env yields config");
        assert!(
            out.contains("HAS_QUOTES = { value = \"a\\\"b\\\\c\", force = false }"),
            "TOML must escape \" and \\; got: {out}"
        );
    }
}
